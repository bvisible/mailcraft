// //// Neoffice — added file (no upstream equivalent): what the hosting page's answer to a renewal of tokens means for
// each mailbox waiting for it, apart from the page and the network so that it is tested natively.

//! When the server refuses a mailbox's token, the browser shell asks the hosting page for new ones (`host::renew`). Its
//! answer gives some mailboxes in full (with a secret), may name others without one yet, and leaves the rest out. Each
//! mailbox waiting for it then signs in again, asks again a little later, or asks for a reload; one started again since
//! it asked sees to itself.
// Used by the page's credentials (host.rs), which only the browser builds; built on every target so that its tests
// run natively.
#![allow(dead_code)]

use std::collections::HashMap;

/// What the page answered.
#[derive(Clone, Default)]
pub struct Answer {
    /// The mailboxes it gives in full: address (lower case) → (secret, Bearer).
    pub given: HashMap<String, (String, bool)>,
    /// Every address it names (lower case), with a secret or not.
    pub named: Vec<String>,
}

/// Counts only: never a secret, nor an address.
impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Answer").field("given", &self.given.len()).field("named", &self.named.len()).finish()
    }
}

/// What a mailbox waiting for the answer does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Next {
    /// Sign in again with the secret the answer gave it (kept with the others by [`merge`]).
    SignIn,
    /// Ask the page again a little later: it had nothing for the mailbox yet.
    AskLater,
    /// Only reloading the page gives it access again.
    Reload,
    /// Started again since it asked (or gone): its new start sees to itself.
    Skip,
}

/// The secrets kept after an answer: those of the mailboxes it gives in full replace theirs, and the others stay, so
/// that an answer naming a mailbox without a secret, or leaving it out, takes nothing from the other mailboxes.
pub fn merge(host: &mut HashMap<String, (String, bool)>, answer: &Answer) {
    for (email, given) in &answer.given {
        host.insert(email.clone(), given.clone());
    }
}

/// What the mailbox `email` does after `answer` (`None`: the page gave none), asked for by its start `asked`, now at
/// start `now` (`None`: gone), having asked `tries` times in a row before, at most `max`.
pub fn next(answer: Option<&Answer>, email: &str, asked: u32, now: Option<u32>, tries: u8, max: u8) -> Next {
    if now != Some(asked) {
        return Next::Skip;
    }
    if answer.is_some_and(|a| a.given.contains_key(email)) {
        return Next::SignIn;
    }
    // Nothing for it yet: no answer, or one that names it without a secret.
    let later = answer.is_none_or(|a| a.named.iter().any(|n| n == email));
    if later && tries < max { Next::AskLater } else { Next::Reload }
}

/// The mailboxes waiting for an answer, each with the start that asked: one asking again (started again meanwhile)
/// waits with its new start.
pub fn wait(waiting: &mut Vec<(String, u32)>, id: &str, generation: u32) {
    match waiting.iter_mut().find(|(w, _)| w == id) {
        Some(entry) => entry.1 = generation,
        None => waiting.push((id.to_string(), generation)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(given: &[(&str, &str)], named: &[&str]) -> Answer {
        Answer {
            given: given.iter().map(|(e, s)| (e.to_string(), (s.to_string(), true))).collect(),
            named: named.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn an_answer_replaces_only_the_mailboxes_it_gives_in_full() {
        let mut host: HashMap<String, (String, bool)> =
            [("a@example.org", "old-a"), ("b@example.org", "old-b")].iter().map(|(e, s)| (e.to_string(), (s.to_string(), true))).collect();
        // Named without a secret, or left out: the other mailbox keeps its own.
        merge(&mut host, &answer(&[("a@example.org", "new-a")], &["a@example.org", "b@example.org"]));
        assert_eq!(host.get("a@example.org").map(|(s, b)| (s.as_str(), *b)), Some(("new-a", true)));
        assert_eq!(host.get("b@example.org").map(|(s, _)| s.as_str()), Some("old-b"));
        merge(&mut host, &answer(&[], &[]));
        assert_eq!(host.len(), 2, "an empty answer takes nothing away");
    }

    #[test]
    fn each_mailbox_waiting_signs_in_asks_later_reloads_or_sees_to_itself() {
        let given = answer(&[("a@example.org", "new-a")], &["a@example.org", "b@example.org"]);
        assert_eq!(next(Some(&given), "a@example.org", 1, Some(1), 0, 60), Next::SignIn);
        // What it signs in with is what the answer gave it, and no secret shows when the answer is printed.
        let mut host = HashMap::new();
        merge(&mut host, &given);
        assert_eq!(host.get("a@example.org").map(|(s, _)| s.as_str()), Some("new-a"));
        assert!(!format!("{given:?}").contains("new-a") && !format!("{given:?}").contains("@"), "{given:?}");
        assert_eq!(next(Some(&given), "b@example.org", 1, Some(1), 0, 60), Next::AskLater, "named without a secret yet");
        assert_eq!(next(Some(&given), "c@example.org", 1, Some(1), 0, 60), Next::Reload, "left out");
        assert_eq!(next(None, "c@example.org", 1, Some(1), 59, 60), Next::AskLater, "no answer: again");
        assert_eq!(next(None, "c@example.org", 1, Some(1), 60, 60), Next::Reload, "after an hour of them");
        // Started again (or removed) since it asked: its new start sees to itself.
        assert_eq!(next(Some(&given), "a@example.org", 1, Some(2), 0, 60), Next::Skip);
        assert_eq!(next(None, "a@example.org", 1, None, 0, 60), Next::Skip);
        // A mailbox started again that asks while the first request is out waits with its new start.
        let mut waiting = Vec::new();
        wait(&mut waiting, "a1", 1);
        wait(&mut waiting, "a2", 1);
        wait(&mut waiting, "a1", 2);
        assert_eq!(waiting, vec![("a1".to_string(), 2), ("a2".to_string(), 1)]);
    }
}
