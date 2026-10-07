//! Memory in process, for the tests of the crates above.

use std::sync::Mutex;

use async_trait::async_trait;

use crate::{Content, Experience, Memory, Person, Result, format_recall};

/// Keeps every capture, once per idempotency key, and recalls by plain word
/// match: what was captured about the person, and what mentions a word of
/// the query.
#[derive(Debug, Default)]
pub struct InMemoryMemory {
    experiences: Mutex<Vec<Experience>>,
}

impl InMemoryMemory {
    /// Everything captured, oldest first.
    pub fn experiences(&self) -> Vec<Experience> {
        self.experiences
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

/// A memory as a recalled line.
fn line(experience: &Experience) -> String {
    match &experience.content {
        Content::Message { text, .. } => format!("- {text}"),
        Content::Json { data } => format!("- {data}"),
    }
}

#[async_trait]
impl Memory for InMemoryMemory {
    async fn capture(&self, experience: Experience) -> Result<()> {
        let mut experiences = self
            .experiences
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !experiences
            .iter()
            .any(|stored| stored.idempotency_key == experience.idempotency_key)
        {
            experiences.push(experience);
        }
        Ok(())
    }

    async fn recall(&self, query: &str, person: Option<&Person>) -> Result<String> {
        let experiences = self.experiences();
        let about: Vec<String> = match person {
            Some(person) => experiences
                .iter()
                .filter(|experience| experience.observed_actor.as_ref() == Some(&person.id))
                .map(line)
                .collect(),
            None => Vec::new(),
        };
        let terms: Vec<String> = query
            .split_whitespace()
            .map(str::to_lowercase)
            .filter(|term| term.chars().count() >= 3)
            .collect();
        let general: Vec<String> = experiences
            .iter()
            .map(line)
            .filter(|line| {
                let line = line.to_lowercase();
                terms.iter().any(|term| line.contains(term.as_str()))
            })
            .collect();
        Ok(format_recall(
            person,
            &about.join("\n"),
            &general.join("\n"),
        ))
    }
}

#[cfg(test)]
#[path = "fake_tests.rs"]
mod tests;
