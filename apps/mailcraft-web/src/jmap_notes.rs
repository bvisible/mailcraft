// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent)
//! A calendar job's notes when it is done again with a renewed credential ([`super::pim`]): what the server answered of
//! the changes of notes before it turned the credential down stays answered, and only the others go again, so that a
//! change of a note is never sent twice.

use mailcraft_model::note::{NoteOp, NoteOutcome, NoteResult, NotesAsk, NotesUploaded};
use mailcraft_model::sync::{JmapPimRequest, JmapPimResult};

/// The notes of a job that may be done more than once.
pub(crate) struct NotesReplay {
    /// What the next try asks of the notes: the changes not answered yet.
    left: Option<NotesAsk>,
    /// What the server answered of the changes, every try together.
    answered: Vec<NoteOutcome>,
    /// The server the job asks (its session): the answer says so, whichever try had the outcomes.
    session: String,
}

impl NotesReplay {
    pub(crate) fn new(request: &JmapPimRequest) -> NotesReplay {
        NotesReplay { left: request.notes.clone(), answered: Vec::new(), session: request.session.clone() }
    }

    /// What the next try asks of the notes.
    pub(crate) fn ask(&self) -> Option<NotesAsk> {
        self.left.clone()
    }

    /// Take what a try answered: the changes it answered don't go again.
    pub(crate) fn take(&mut self, answer: &JmapPimResult) {
        let now: Vec<NoteOutcome> =
            answer.notes.as_ref().and_then(|notes| notes.uploaded.as_ref()).map(|uploaded| uploaded.outcomes.clone()).unwrap_or_default();
        if let Some(changes) = self.left.as_mut().and_then(|ask| ask.changes.as_mut()) {
            changes.ops.retain(|op| !now.iter().any(|outcome| same(&outcome.op, op)));
        }
        // A change the server didn't take as it was sent (stale, gone, refused) leaves the window's notes other than the
        // server's: the next try reads them whole, whatever the window asked and whatever state it held.
        if now.iter().any(|outcome| !matches!(outcome.result, NoteResult::Done | NoteResult::Failed))
            && let Some(ask) = self.left.as_mut()
        {
            ask.read = true;
            ask.state.clear();
            ask.read_at = 0;
        }
        self.answered.extend(now);
    }

    /// `answer` with every outcome of the changes of notes answered so far, whichever try had them.
    pub(crate) fn give(&self, mut answer: JmapPimResult) -> JmapPimResult {
        if self.answered.is_empty() {
            return answer;
        }
        let (account, email) = (answer.account, answer.email.clone());
        let notes = answer.notes.get_or_insert_with(Default::default);
        let last = notes.uploaded.take().unwrap_or_default();
        notes.uploaded = Some(NotesUploaded { account, email, session: self.session.clone(), outcomes: self.answered.clone(), ..last });
        answer
    }
}

/// Whether two changes are the same one: an outcome carries the change it is of, as the host tidied it.
fn same(answered: &NoteOp, asked: &NoteOp) -> bool {
    let mut asked = asked.clone();
    asked.tidy();
    *answered == asked
}

#[cfg(test)]
mod tests {
    use mailcraft_model::note::{NoteChangeKind, NoteFields, NotesAnswer, NotesDownload, NotesUpload};

    use super::*;

    const ME: &str = "ada@example.org";

    fn op(kind: NoteChangeKind, note: u64, remote: &str) -> NoteOp {
        NoteOp {
            kind,
            note,
            remote: remote.into(),
            token: format!("mark-{note}"),
            fields: NoteFields { title: Some("x".into()), ..Default::default() },
            ..Default::default()
        }
    }

    fn done(op: &NoteOp) -> NoteOutcome {
        let mut op = op.clone();
        op.tidy();
        NoteOutcome { op, result: NoteResult::Done, ..Default::default() }
    }

    fn answered(outcomes: Vec<NoteOutcome>, token_refused: bool) -> JmapPimResult {
        let uploaded = NotesUploaded { account: 7, email: ME.into(), outcomes, token_refused, ..Default::default() };
        JmapPimResult {
            account: 7,
            email: ME.into(),
            auth_refused: token_refused,
            notes: Some(NotesAnswer { uploaded: Some(uploaded), ..Default::default() }),
            ..Default::default()
        }
    }

    #[test]
    fn what_a_try_answered_does_not_go_again_and_the_answer_has_every_outcome() {
        let ops = vec![op(NoteChangeKind::Create, 1, ""), op(NoteChangeKind::Update, 2, "VN-2"), op(NoteChangeKind::Delete, 3, "VN-6")];
        let ask = NotesAsk {
            read: true,
            state: "s1".into(),
            changes: Some(NotesUpload { account: 7, email: ME.into(), ops: ops.clone() }),
            ..Default::default()
        };
        let request = JmapPimRequest { account: 7, email: ME.into(), notes: Some(ask), ..Default::default() };
        let mut replay = NotesReplay::new(&request);
        assert_eq!(replay.ask(), request.notes);
        // The first try: two changes answered, then the credential turned down.
        replay.take(&answered(vec![done(&ops[0]), done(&ops[1])], true));
        let next = replay.ask().unwrap();
        assert_eq!(next.changes.map(|changes| changes.ops), Some(vec![ops[2].clone()]), "only the third goes again");
        assert!(next.read && next.state == "s1", "and the notes are read as asked");
        // The second, with a new credential: the third answered, and the notes read.
        let mut second = answered(vec![done(&ops[2])], false);
        second.notes.as_mut().unwrap().download = Some(NotesDownload { account: 7, email: ME.into(), offered: true, ..Default::default() });
        replay.take(&second);
        let given = replay.give(second);
        let uploaded = given.notes.as_ref().and_then(|notes| notes.uploaded.as_ref()).unwrap();
        assert_eq!(uploaded.outcomes.iter().map(|o| o.op.note).collect::<Vec<_>>(), [1, 2, 3]);
        assert!(!uploaded.token_refused && given.notes.as_ref().is_some_and(|notes| notes.download.is_some()));
        // An answer that says nothing of the notes (the sign-in failed again) still has what was answered.
        let failed = JmapPimResult { account: 7, email: ME.into(), auth_refused: true, ..Default::default() };
        let given = replay.give(failed);
        assert_eq!(given.notes.and_then(|notes| notes.uploaded).map(|uploaded| uploaded.outcomes.len()), Some(3));
        // Which server they are of goes with them, whichever try had them, even one that said nothing of the notes.
        let request = JmapPimRequest { session: "https://mail.example.org/jmap/session".into(), ..request };
        let mut replay = NotesReplay::new(&request);
        replay.take(&answered(vec![done(&ops[0])], true));
        let failed = JmapPimResult { account: 7, email: ME.into(), auth_refused: true, ..Default::default() };
        let given = replay.give(failed);
        assert_eq!(given.notes.and_then(|notes| notes.uploaded).map(|uploaded| uploaded.session), Some(request.session.clone()));
        // Nothing answered: the answer as it came.
        let untouched = NotesReplay::new(&JmapPimRequest::default()).give(JmapPimResult::default());
        assert!(untouched.notes.is_none());
    }

    #[test]
    fn a_change_a_try_did_not_have_taken_as_sent_has_the_next_try_read_the_notes_whole() {
        let ops = vec![op(NoteChangeKind::Update, 2, "VN-2"), op(NoteChangeKind::Update, 3, "VN-3")];
        let ask = NotesAsk {
            read: false,
            state: "s1".into(),
            changes: Some(NotesUpload { account: 7, email: ME.into(), ops: ops.clone() }),
            ..Default::default()
        };
        let mut replay = NotesReplay::new(&JmapPimRequest { account: 7, email: ME.into(), notes: Some(ask), ..Default::default() });
        // The first try: the server had other words (stale), then turned the credential down.
        let mut stale = done(&ops[0]);
        stale.result = NoteResult::Stale;
        replay.take(&answered(vec![stale], true));
        let next = replay.ask().unwrap();
        assert!(next.read && next.state.is_empty(), "the notes are read whole: the window's are other than the server's");
        assert_eq!(next.changes.map(|changes| changes.ops.len()), Some(1));
    }
}
