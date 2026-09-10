//! Статический контракт workflow graph и границы повторяемого участка; здесь нет durable-состояния и исполнения Steps.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::{Dependencies, Dependency, Outputs};

#[derive(Clone, Copy)]
pub(crate) struct GraphStep<'a> {
    pub(crate) id: &'a str,
    pub(crate) depends_on: &'a Dependencies,
    pub(crate) outputs: &'a Outputs,
}

#[derive(Clone, Debug)]
pub(crate) struct Repeat {
    pub(crate) entry: usize,
    pub(crate) decision: usize,
    pub(crate) members: Vec<usize>,
}

pub(crate) struct Graph<'a> {
    pub(crate) steps: Vec<GraphStep<'a>>,
    pub(crate) repeat: Option<Repeat>,
    pub(crate) indices: HashMap<&'a str, usize>,
}

impl<'a> Graph<'a> {
    /// Проверяет достижимость и единственную границу возврата; см. Rules в `features/conditional_graph.feature`.
    pub(crate) fn validate(steps: Vec<GraphStep<'a>>) -> Result<Self, String> {
        if steps.is_empty() {
            return Err("steps должен быть непустым".to_owned());
        }
        let indices = steps
            .iter()
            .enumerate()
            .map(|(index, step)| (step.id, index))
            .collect::<HashMap<_, _>>();
        let mut graph = Self {
            steps,
            indices,
            repeat: None,
        };
        graph.validate_references()?;
        graph.validate_unambiguous_dependencies()?;
        graph.validate_reachability()?;
        graph.repeat = graph.validate_repeat()?;
        graph.validate_branch_reachability()?;
        Ok(graph)
    }

    fn validate_references(&self) -> Result<(), String> {
        for step in &self.steps {
            for leaf in step.depends_on.alternatives().into_iter().flatten() {
                let source = self
                    .indices
                    .get(leaf.step())
                    .map(|index| &self.steps[*index])
                    .ok_or_else(|| {
                        format!(
                            "Step '{}': depends-on ссылается на неизвестный Step '{}'",
                            step.id,
                            leaf.step()
                        )
                    })?;
                if let Some(output) = leaf.output()
                    && !source.outputs.iter().any(|id| id == output)
                {
                    return Err(format!(
                        "Step '{}': неизвестный output '{}:{output}'",
                        step.id,
                        leaf.step()
                    ));
                }
            }
            if !step
                .depends_on
                .alternatives()
                .iter()
                .any(|leaves| self.coherent(leaves))
            {
                return Err(format!(
                    "Step '{}': несовместимые outputs в depends-on",
                    step.id
                ));
            }
        }
        Ok(())
    }

    fn validate_unambiguous_dependencies(&self) -> Result<(), String> {
        for step in &self.steps {
            let alternatives = step.depends_on.alternatives();
            for (index, left) in alternatives.iter().enumerate() {
                let left_sources = ordered_sources(left);
                for right in alternatives.iter().skip(index + 1) {
                    if left_sources == ordered_sources(right) {
                        let combined = left.iter().chain(right).copied().collect::<Vec<_>>();
                        if self.coherent(&combined) {
                            return Err(format!(
                                "Step '{}': одновременно выполнимые alternatives выбирают одинаковую input group",
                                step.id
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn coherent(&self, leaves: &[&Dependency]) -> bool {
        leaves.iter().all(|leaf| {
            self.steps[self.indices[leaf.step()]]
                .outputs
                .variants()
                .iter()
                .any(|variant| {
                    leaves
                        .iter()
                        .filter(|other| other.step() == leaf.step())
                        .all(|other| other.output().is_none_or(|output| variant.contains(output)))
                })
        })
    }

    fn validate_reachability(&self) -> Result<(), String> {
        let mut reachable = vec![false; self.steps.len()];
        reachable[0] = true;
        loop {
            let mut changed = false;
            for (index, step) in self.steps.iter().enumerate().skip(1) {
                if !reachable[index]
                    && !step.depends_on.is_empty()
                    && step.depends_on.alternatives().iter().any(|leaves| {
                        self.coherent(leaves)
                            && leaves
                                .iter()
                                .all(|leaf| reachable[self.indices[leaf.step()]])
                    })
                {
                    reachable[index] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        if let Some(index) = reachable.iter().position(|value| !value) {
            return Err(format!(
                "Step '{}': статически недостижим из initial activation",
                self.steps[index].id
            ));
        }
        Ok(())
    }

    fn validate_repeat(&self) -> Result<Option<Repeat>, String> {
        let count = self.steps.len();
        let mut paths = vec![vec![false; count]; count];
        for (target, step) in self.steps.iter().enumerate() {
            for source in step.depends_on {
                paths[self.indices[source.as_str()]][target] = true;
            }
        }
        for via in 0..count {
            for source in 0..count {
                for target in 0..count {
                    paths[source][target] |= paths[source][via] && paths[via][target];
                }
            }
        }
        let Some(entry) = (0..count).find(|index| paths[*index][*index]) else {
            return Ok(None);
        };
        let members = (0..count)
            .filter(|index| paths[entry][*index] && paths[*index][entry])
            .collect::<Vec<_>>();
        if (0..count).any(|index| paths[index][index] && !members.contains(&index)) {
            return Err("два независимых циклических участка запрещены".to_owned());
        }
        let decisions = self.steps[entry]
            .depends_on
            .iter()
            .map(|id| self.indices[id.as_str()])
            .filter(|index| members.contains(index))
            .collect::<Vec<_>>();
        let [decision] = decisions.as_slice() else {
            return Err("повторяемый участок должен иметь одну точку решения; параллельные возвраты запрещены".to_owned());
        };
        let mut visited = Vec::new();
        loop {
            let previous = visited.len();
            for &index in &members {
                if !visited.contains(&index)
                    && self.steps[index].depends_on.iter().all(|id| {
                        let source = self.indices[id.as_str()];
                        !members.contains(&source)
                            || (index == entry && source == *decision)
                            || visited.contains(&source)
                    })
                {
                    visited.push(index);
                }
            }
            if previous == visited.len() {
                break;
            }
        }
        if visited.len() != members.len() {
            return Err("вложенный цикл или дополнительный возврат запрещён".to_owned());
        }
        for (target, step) in self.steps.iter().enumerate() {
            if !members.contains(&target)
                && step.depends_on.iter().any(|id| {
                    let source = self.indices[id.as_str()];
                    members.contains(&source) && source != *decision
                })
            {
                return Err("ранний возврат: ветвь покидает участок до точки решения".to_owned());
            }
        }
        Ok(Some(Repeat {
            entry,
            decision: *decision,
            members,
        }))
    }

    fn validate_branch_reachability(&self) -> Result<(), String> {
        type Choices = BTreeMap<usize, BTreeSet<String>>;
        let mut paths: Vec<Vec<Choices>> = vec![Vec::new(); self.steps.len()];
        paths[0] = self.steps[0]
            .outputs
            .variants()
            .into_iter()
            .map(|outputs| BTreeMap::from([(0, outputs)]))
            .collect();
        loop {
            let mut changed = false;
            for (index, step) in self.steps.iter().enumerate().skip(1) {
                for leaves in step.depends_on.alternatives() {
                    let mut candidates = vec![Choices::new()];
                    for leaf in leaves {
                        let source = self.indices[leaf.step()];
                        let mut merged = Vec::new();
                        for candidate in &candidates {
                            for path in &paths[source] {
                                if path.contains_key(&index)
                                    || leaf.output().is_some_and(|id| !path[&source].contains(id))
                                    || candidate.iter().any(|(step, outputs)| {
                                        path.get(step).is_some_and(|other| other != outputs)
                                    })
                                {
                                    continue;
                                }
                                let mut result = candidate.clone();
                                result.extend(path.clone());
                                if !merged.contains(&result) {
                                    merged.push(result);
                                }
                            }
                        }
                        candidates = merged;
                    }
                    for candidate in candidates {
                        for outputs in step.outputs.variants() {
                            let mut result = candidate.clone();
                            result.insert(index, outputs);
                            if !paths[index].contains(&result) {
                                paths[index].push(result);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        if let Some(index) = paths.iter().position(Vec::is_empty) {
            return Err(format!(
                "Step '{}': статически недостижим при согласованном выборе ветвей",
                self.steps[index].id
            ));
        }
        Ok(())
    }

    pub(crate) fn guarantees(
        &self,
        dependencies: &Dependencies,
        source: &str,
        output: &str,
    ) -> bool {
        let Some(&index) = self.indices.get(source) else {
            return false;
        };
        dependencies
            .alternatives()
            .iter()
            .filter(|leaves| self.coherent(leaves))
            .all(|leaves| {
                leaves.iter().any(|leaf| leaf.step() == source)
                    && self.steps[index]
                        .outputs
                        .variants()
                        .iter()
                        .filter(|variant| {
                            leaves
                                .iter()
                                .filter(|leaf| leaf.step() == source)
                                .all(|leaf| leaf.output().is_none_or(|id| variant.contains(id)))
                        })
                        .all(|variant| variant.contains(output))
            })
    }
}

fn ordered_sources<'a>(leaves: &[&'a Dependency]) -> Vec<&'a str> {
    let mut sources = Vec::new();
    for leaf in leaves {
        if !sources.contains(&leaf.step()) {
            sources.push(leaf.step());
        }
    }
    sources
}
