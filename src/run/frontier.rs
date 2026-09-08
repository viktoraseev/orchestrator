//! Вычисление готовности из зафиксированных inputs и опубликованных artifacts; модуль не запускает executors и не изменяет storage.

use std::collections::HashMap;

use crate::config::CommandError;
use crate::domain::{Dependency, DurableAttempt, Expression, Graph, MaterializedWorkflow};

pub(super) struct Frontier {
    pub(super) ready: Vec<usize>,
    pub(super) inputs: HashMap<usize, Vec<u64>>,
    pub(super) missing: Vec<String>,
}

#[derive(Clone)]
enum Inputs {
    Ready(Vec<u64>),
    Waiting { present: bool, missing: Vec<String> },
    Closed,
}

#[derive(Clone)]
enum Resolution {
    Completed(usize),
    Pending(Inputs),
    Closed,
}

struct Evaluation<'a> {
    graph: Graph<'a>,
    attempts: &'a [DurableAttempt],
    states: Vec<Option<Resolution>>,
    epoch: Option<u64>,
}

/// Восстанавливает frontier без счётчика итераций: номер последнего входа задаёт текущее выполнение участка; см. Rule «Повторяемый участок имеет один вход и одну точку решения после всех выбранных ветвей» в `features/conditional_graph.feature`.
pub(super) fn compute_frontier(
    workflow: &MaterializedWorkflow,
    attempts: &[DurableAttempt],
) -> Result<Frontier, CommandError> {
    let graph = workflow
        .graph()
        .map_err(|context| CommandError::Invalid { context })?;
    if attempts.is_empty() {
        return Ok(Frontier {
            ready: vec![0],
            inputs: HashMap::from([(0, Vec::new())]),
            missing: Vec::new(),
        });
    }
    let epoch = graph.repeat.as_ref().and_then(|region| {
        attempts
            .iter()
            .filter(|attempt| attempt.step_index == region.entry)
            .map(|attempt| attempt.number)
            .max()
    });
    let mut evaluation = Evaluation {
        graph,
        attempts,
        states: vec![None; workflow.steps.len()],
        epoch,
    };
    for index in 0..workflow.steps.len() {
        evaluation.step(index);
    }
    let repeat_input = evaluation.next_repeat();
    let mut frontier = Frontier {
        ready: Vec::new(),
        inputs: HashMap::new(),
        missing: Vec::new(),
    };
    for (index, state) in evaluation.states.iter().enumerate() {
        let input = if repeat_input
            .as_ref()
            .is_some_and(|(entry, _)| *entry == index)
        {
            repeat_input.as_ref().map(|(_, input)| input.clone())
        } else if let Some(Resolution::Pending(Inputs::Ready(input))) = state {
            Some(input.clone())
        } else {
            None
        };
        if let Some(input) = input {
            frontier.ready.push(index);
            frontier.inputs.insert(index, input);
        }
        if let Some(Resolution::Pending(Inputs::Waiting {
            present: true,
            missing,
        })) = state
        {
            for source in missing {
                if !frontier.missing.contains(source) {
                    frontier.missing.push(source.clone());
                }
            }
        }
    }
    Ok(frontier)
}

impl Evaluation<'_> {
    fn latest(&self, index: usize) -> Option<usize> {
        let in_body = self
            .graph
            .repeat
            .as_ref()
            .is_some_and(|region| region.members.contains(&index) && region.entry != index);
        self.attempts
            .iter()
            .enumerate()
            .filter(|(_, attempt)| {
                attempt.step_index == index
                    && (!in_body || self.epoch.is_some_and(|epoch| attempt.number >= epoch))
            })
            .max_by_key(|(_, attempt)| attempt.number)
            .map(|(position, _)| position)
    }

    fn step(&mut self, index: usize) -> Resolution {
        if let Some(state) = &self.states[index] {
            return state.clone();
        }
        let latest = self.latest(index);
        let entry = self
            .graph
            .repeat
            .as_ref()
            .is_some_and(|region| region.entry == index);
        if let Some(position) = latest {
            if !self.attempts[position].record.is_completed() {
                return self.store(
                    index,
                    Resolution::Pending(Inputs::Waiting {
                        present: false,
                        missing: vec![self.graph.steps[index].id.to_owned()],
                    }),
                );
            }
            if entry || self.graph.steps[index].depends_on.is_empty() {
                return self.store(index, Resolution::Completed(position));
            }
        }
        let tree = &self.graph.steps[index].depends_on.tree.clone();
        let inputs = self.all(tree, index);
        let state = self.select(index, inputs);
        let is_decision = self
            .graph
            .repeat
            .as_ref()
            .is_some_and(|region| region.decision == index && region.entry != index);
        if is_decision && matches!(state, Resolution::Pending(Inputs::Ready(_))) {
            let members = self
                .graph
                .repeat
                .as_ref()
                .map_or_else(Vec::new, |region| region.members.clone());
            let mut missing = Vec::new();
            for member in members {
                if member != index && matches!(self.step(member), Resolution::Pending(_)) {
                    missing.push(self.graph.steps[member].id.to_owned());
                }
            }
            if !missing.is_empty() {
                return self.store(
                    index,
                    Resolution::Pending(Inputs::Waiting {
                        present: true,
                        missing,
                    }),
                );
            }
        }
        self.store(index, state)
    }

    fn store(&mut self, index: usize, state: Resolution) -> Resolution {
        self.states[index] = Some(state.clone());
        state
    }

    fn select(&self, index: usize, options: Vec<Inputs>) -> Resolution {
        let mut waiting = None;
        for option in options {
            match option {
                Inputs::Ready(input) => {
                    return if let Some((position, _)) =
                        self.attempts.iter().enumerate().find(|(_, attempt)| {
                            attempt.step_index == index && attempt.record.input() == input
                        }) {
                        Resolution::Completed(position)
                    } else {
                        Resolution::Pending(Inputs::Ready(input))
                    };
                }
                Inputs::Waiting { .. } => {
                    if waiting.is_none() {
                        waiting = Some(option);
                    }
                }
                Inputs::Closed => {}
            }
        }
        if let Some(waiting) = waiting {
            Resolution::Pending(waiting)
        } else {
            Resolution::Closed
        }
    }

    fn next_repeat(&mut self) -> Option<(usize, Vec<u64>)> {
        let region = self.graph.repeat.as_ref()?.clone();
        self.epoch?;
        if region
            .members
            .iter()
            .any(|index| matches!(self.states[*index], Some(Resolution::Pending(_))))
        {
            return None;
        }
        if !matches!(self.states[region.decision], Some(Resolution::Completed(_))) {
            return None;
        }
        let tree = self.graph.steps[region.entry].depends_on.tree.clone();
        let choices = self.all(&tree, region.entry);
        match self.select(region.entry, choices) {
            Resolution::Pending(Inputs::Ready(input)) => Some((region.entry, input)),
            _ => None,
        }
    }

    fn all(&mut self, tree: &[Expression<Dependency>], target: usize) -> Vec<Inputs> {
        let mut result = vec![Inputs::Ready(Vec::new())];
        for node in tree {
            let choices = self.expression(node, target);
            result = result
                .into_iter()
                .flat_map(|prefix| choices.iter().map(move |choice| merge(&prefix, choice)))
                .collect();
        }
        result
    }

    fn expression(&mut self, expression: &Expression<Dependency>, target: usize) -> Vec<Inputs> {
        match expression {
            Expression::Leaf(leaf) => vec![self.leaf(leaf, target)],
            Expression::All { all } => self.all(all, target),
            Expression::OneOf { one_of } => {
                let mut choices = one_of
                    .iter()
                    .flat_map(|node| self.expression(node, target))
                    .collect::<Vec<_>>();
                choices.sort_by_key(|choice| {
                    std::cmp::Reverse(match choice {
                        Inputs::Ready(input) => input.iter().max().map(|number| (true, *number)),
                        _ => None,
                    })
                });
                choices
            }
        }
    }

    fn leaf(&mut self, leaf: &Dependency, target: usize) -> Inputs {
        let source = self.graph.indices[leaf.step()];
        if self.epoch.is_none()
            && self
                .graph
                .repeat
                .as_ref()
                .is_some_and(|region| target == region.entry && region.members.contains(&source))
        {
            // 2026-09-08 12:51 До первого входа feedback ещё отсутствует, но выбранная внешняя часть той же обязательной группы должна сделать run blocked, а не completed.
            return Inputs::Waiting {
                present: false,
                missing: vec![leaf.step().to_owned()],
            };
        }
        match self.step(source) {
            Resolution::Completed(position) => {
                let attempt = &self.attempts[position];
                if leaf
                    .output()
                    .is_some_and(|id| !attempt.outputs.iter().any(|output| output == id))
                {
                    Inputs::Closed
                } else {
                    Inputs::Ready(vec![attempt.number])
                }
            }
            Resolution::Closed => Inputs::Closed,
            Resolution::Pending(_) => Inputs::Waiting {
                present: false,
                missing: vec![leaf.step().to_owned()],
            },
        }
    }
}

fn merge(left: &Inputs, right: &Inputs) -> Inputs {
    match (left, right) {
        (Inputs::Closed, _) | (_, Inputs::Closed) => Inputs::Closed,
        (Inputs::Ready(left), Inputs::Ready(right)) => {
            let mut input = left.clone();
            for number in right {
                if !input.contains(number) {
                    input.push(*number);
                }
            }
            Inputs::Ready(input)
        }
        _ => {
            let mut present = false;
            let mut missing = Vec::new();
            for part in [left, right] {
                match part {
                    Inputs::Ready(input) => present |= !input.is_empty(),
                    Inputs::Waiting {
                        present: available,
                        missing: sources,
                    } => {
                        present |= available;
                        missing.extend(sources.clone());
                    }
                    Inputs::Closed => {}
                }
            }
            Inputs::Waiting { present, missing }
        }
    }
}
