//! Выражения опубликованных outputs и согласованных dependencies; модуль не выбирает момент запуска и не читает durable-файлы.

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use super::SymbolicId;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Expression<T> {
    Leaf(T),
    All {
        all: Vec<Self>,
    },
    OneOf {
        #[serde(rename = "one-of")]
        one_of: Vec<Self>,
    },
}

impl<T> Expression<T> {
    fn leaves<'a>(&'a self, result: &mut Vec<&'a T>) {
        match self {
            Self::Leaf(leaf) => result.push(leaf),
            Self::All { all: children } | Self::OneOf { one_of: children } => {
                for child in children {
                    child.leaves(result);
                }
            }
        }
    }

    fn validate_groups(&self) -> Result<(), String> {
        match self {
            Self::Leaf(_) => Ok(()),
            Self::All { all: children } | Self::OneOf { one_of: children } => {
                if children.is_empty() {
                    return Err("пустая группа all/one-of".to_owned());
                }
                children.iter().try_for_each(Self::validate_groups)
            }
        }
    }

    pub(crate) fn alternatives(&self) -> Vec<Vec<&T>> {
        match self {
            Self::Leaf(leaf) => vec![vec![leaf]],
            Self::All { all } => alternatives(all),
            Self::OneOf { one_of } => one_of.iter().flat_map(Self::alternatives).collect(),
        }
    }
}

pub(crate) fn alternatives<T>(expressions: &[Expression<T>]) -> Vec<Vec<&T>> {
    let mut result = vec![Vec::new()];
    for expression in expressions {
        let choices = expression.alternatives();
        result = result
            .into_iter()
            .flat_map(|prefix| {
                choices.iter().map(move |choice| {
                    let mut candidate = prefix.clone();
                    candidate.extend(choice);
                    candidate
                })
            })
            .collect();
    }
    result
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Dependency {
    Step(String),
    Artifact { step: String, output: String },
}

impl Dependency {
    pub(crate) fn step(&self) -> &str {
        match self {
            Self::Step(step) | Self::Artifact { step, .. } => step,
        }
    }

    pub(crate) fn output(&self) -> Option<&str> {
        match self {
            Self::Step(_) => None,
            Self::Artifact { output, .. } => Some(output),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "Vec<Expression<String>>", into = "Vec<Expression<String>>")]
pub(crate) struct Outputs {
    tree: Vec<Expression<String>>,
    leaves: Vec<String>,
}

impl TryFrom<Vec<Expression<String>>> for Outputs {
    type Error = String;
    fn try_from(tree: Vec<Expression<String>>) -> Result<Self, Self::Error> {
        let mut leaves = Vec::new();
        for expression in &tree {
            expression.validate_groups()?;
            expression.leaves(&mut leaves);
        }
        let mut seen = HashSet::new();
        for leaf in &leaves {
            SymbolicId::parse("InputId", leaf)?;
            if !seen.insert(*leaf) {
                return Err("outputs содержит повтор".to_owned());
            }
        }
        let leaves = leaves.into_iter().cloned().collect();
        Ok(Self { tree, leaves })
    }
}

impl From<Outputs> for Vec<Expression<String>> {
    fn from(value: Outputs) -> Self {
        value.tree
    }
}

impl From<Vec<String>> for Outputs {
    fn from(leaves: Vec<String>) -> Self {
        Self {
            tree: leaves.iter().cloned().map(Expression::Leaf).collect(),
            leaves,
        }
    }
}

impl<'a> IntoIterator for &'a Outputs {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.leaves.iter()
    }
}

impl Outputs {
    pub(crate) fn ids(&self) -> &[String] {
        &self.leaves
    }
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, String> {
        self.leaves.iter()
    }
    pub(crate) fn len(&self) -> usize {
        self.leaves.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }
    pub(crate) fn contains(&self, id: &String) -> bool {
        self.leaves.contains(id)
    }

    pub(crate) fn variants(&self) -> Vec<BTreeSet<String>> {
        alternatives(&self.tree)
            .into_iter()
            .map(|leaves| leaves.into_iter().cloned().collect())
            .collect()
    }

    pub(crate) fn accepts<'a>(&self, outputs: impl IntoIterator<Item = &'a str>) -> bool {
        let actual: BTreeSet<String> = outputs.into_iter().map(str::to_owned).collect();
        self.variants().iter().any(|variant| variant == &actual)
    }

    pub(crate) fn text(&self) -> String {
        expression_text(&self.tree, Clone::clone)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(
    try_from = "Vec<Expression<Dependency>>",
    into = "Vec<Expression<Dependency>>"
)]
pub(crate) struct Dependencies {
    pub(crate) tree: Vec<Expression<Dependency>>,
    sources: Vec<String>,
}

impl TryFrom<Vec<Expression<Dependency>>> for Dependencies {
    type Error = String;
    fn try_from(tree: Vec<Expression<Dependency>>) -> Result<Self, Self::Error> {
        let mut leaves = Vec::new();
        for expression in &tree {
            expression.validate_groups()?;
            expression.leaves(&mut leaves);
        }
        let mut sources = Vec::new();
        let mut plain = HashSet::new();
        for node in &tree {
            if let Expression::Leaf(Dependency::Step(step)) = node
                && !plain.insert(step)
            {
                return Err("depends-on содержит повтор".to_owned());
            }
        }
        for leaf in leaves {
            SymbolicId::parse("StepId", leaf.step())?;
            if let Some(output) = leaf.output() {
                SymbolicId::parse("InputId", output)?;
            }

            if !sources.iter().any(|source| source == leaf.step()) {
                sources.push(leaf.step().to_owned());
            }
        }
        Ok(Self { tree, sources })
    }
}

impl From<Dependencies> for Vec<Expression<Dependency>> {
    fn from(value: Dependencies) -> Self {
        value.tree
    }
}

impl<'a> IntoIterator for &'a Dependencies {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.sources.iter()
    }
}

impl Dependencies {
    pub(crate) fn sources(&self) -> &[String] {
        &self.sources
    }
    pub(crate) fn iter(&self) -> std::slice::Iter<'_, String> {
        self.sources.iter()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub(crate) fn alternatives(&self) -> Vec<Vec<&Dependency>> {
        alternatives(&self.tree)
    }
    pub(crate) fn text(&self) -> String {
        expression_text(&self.tree, |leaf| match leaf {
            Dependency::Step(step) => step.clone(),
            Dependency::Artifact { step, output } => format!("{step}:{output}"),
        })
    }
}

fn expression_text<T>(tree: &[Expression<T>], leaf: impl Fn(&T) -> String + Copy) -> String {
    if tree.is_empty() {
        return "-".to_owned();
    }
    tree.iter()
        .map(|node| match node {
            Expression::Leaf(value) => leaf(value),
            Expression::All { all } => format!("all({})", expression_text(all, leaf)),
            Expression::OneOf { one_of } => format!("one-of({})", expression_text(one_of, leaf)),
        })
        .collect::<Vec<_>>()
        .join(",")
}
