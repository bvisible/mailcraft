// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
// //// Neoffice — added file (no upstream equivalent): nothing of a hosting page's calendar is kept "on this computer".

//! The calendar of a tab of a hosting page ([`Session::keeps_local`] false): what its servers have, and nothing beside it.
//!
//! - **The same event under another name** ([`same_by_uid`], every JMAP account, wherever it is kept). A server may call an
//!   event otherwise from one answer to the next (Stalwart named the events of a period by the answer they were in), where
//!   its `uid` is the same in every one: an event a read has under a name that is not known here, with the `uid` of one whose
//!   name the read doesn't have, is that one, and everything that waits about it is about it under its new name.
//! - **Nothing on this computer.** Upstream keeps what a server refused, or took and doesn't list, in a calendar of the
//!   mailbox's own (« On this computer »); in such a tab that calendar would be a second one beside the person's, kept by
//!   nobody. The event stays in the calendar it was made in instead, [`unsent`]: nothing of it goes by itself, a change of it
//!   sends it again ([`send_again`]), and a read that has its `uid` after all adopts it.
//! - **What is there already** ([`bring_home`]): the doubles and the calendars a window of before left in the browser's copy
//!   of the mailbox. A double of an event a server lists (the same `uid`) goes; what the person changed in it since goes to
//!   the server's event as a change; anything else goes to the account's calendar, and to its server (a meeting with people
//!   invited waits there, unsent: nobody is written to without the person's yes). The emptied calendars go.

use mailcraft_model::remote::{EventChange, EventChangeKind, EventsDownload, RemoteEventKind};
use mailcraft_model::{Event, Id, Mailbox, Origin};
use mailcraft_time::Timestamp;

use super::remote;
use crate::Session;

/// The events of `download` that are here under another name take their new one, with all that waits about them: what was
/// made, changed, moved or deleted here is looked for under it from now on.
pub(crate) fn same_by_uid(mailbox: &mut Mailbox, account: Id, calendar: Id, download: &EventsDownload) {
    if !mailbox.is_jmap(account) {
        return;
    }
    let there: std::collections::HashSet<String> = download.events.iter().map(|remote| remote.id.trim().to_string()).collect();
    for remote in &download.events {
        let (name, uid) = (remote.id.trim(), remote.uid.trim());
        let series = remote.kind == RemoteEventKind::SeriesMaster;
        // (An occurrence has its series' uid: it is told from the series.)
        if name.is_empty() || uid.is_empty() || !series && (remote.kind != RemoteEventKind::Single || !remote.series.is_empty()) {
            continue;
        }
        let of_account = |event: &Event| event.origin.account == account;
        if mailbox.events.values().any(|event| of_account(event) && event.origin.id == name) {
            continue;
        }
        let here = mailbox
            .events
            .values()
            .filter(|event| of_account(event) && event.calendar == calendar && event.uid.trim() == uid)
            .filter(|event| event.origin.series.is_empty() && event.recurrence.is_some() == series)
            .filter(|event| !there.contains(&event.origin.id) && (!event.origin.id.is_empty() || unsent(mailbox, event)))
            .map(|event| event.id)
            .min();
        if let Some(id) = here {
            let old = mailbox.event(id).map(|event| event.origin.id.clone()).unwrap_or_default();
            if let Ok(event) = mailbox.event_mut(id) {
                event.origin.id = name.to_string();
            }
            if !old.is_empty() {
                renamed(mailbox, account, &old, name);
            }
            log::info!("calendar: an event the server calls otherwise now is the one that was here");
            continue;
        }
        // Deleted here, and the server yet to hear (or to show it): it stays deleted, under its new name too.
        let gone = mailbox.event_changes.iter().find_map(|change| gone_as(change.account == account, change.kind, change.base.as_deref(), uid));
        let gone =
            gone.or_else(|| mailbox.event_checks.iter().find_map(|check| gone_as(check.account == account, check.kind, check.base.as_deref(), uid)));
        if let Some(old) = gone.filter(|old| !there.contains(old)) {
            renamed(mailbox, account, &old, name);
            delete_as(mailbox, account, name);
        }
    }
}

/// The deletion of the event the server calls `name` now, which it answered it had no such event for under the name it had:
/// it is told again, under this one.
fn delete_as(mailbox: &mut Mailbox, account: Id, name: &str) {
    let answered = |check: &&mailcraft_model::remote::EventCheck| {
        check.account == account && check.kind == EventChangeKind::Delete && check.not_found && !check.of_occurrence() && check.remote == name
    };
    let Some(check) = mailbox.event_checks.iter().find(answered).cloned() else { return };
    mailbox.event_checks.retain(|other| !(other.account == account && other.kind == EventChangeKind::Delete && other.remote == name));
    if !mailbox.event_changes.iter().any(|change| change.account == account && change.remote == name) {
        mailbox.event_changes.push(EventChange {
            account,
            kind: EventChangeKind::Delete,
            event: check.event,
            remote: name.to_string(),
            base: check.base,
            ..Default::default()
        });
    }
}

/// What the server called the event of `uid` that was deleted here, when this is what waits or is looked for.
fn gone_as(ours: bool, kind: EventChangeKind, base: Option<&Event>, uid: &str) -> Option<String> {
    let base = base.filter(|base| ours && kind == EventChangeKind::Delete && base.uid.trim() == uid && !base.origin.id.is_empty())?;
    Some(base.origin.id.clone())
}

/// All that waits about the event (or the series) `old` of `account` is about `name`.
fn renamed(mailbox: &mut Mailbox, account: Id, old: &str, name: &str) {
    let put = |field: &mut String| {
        if field.as_str() == old {
            *field = name.to_string();
        }
    };
    for change in mailbox.event_changes.iter_mut().filter(|change| change.account == account) {
        put(&mut change.remote);
        put(&mut change.series);
        put(&mut change.replaces);
        put(&mut change.moved);
        if let Some(base) = change.base.as_deref_mut() {
            put(&mut base.origin.id);
            put(&mut base.origin.series);
        }
    }
    for check in mailbox.event_checks.iter_mut().filter(|check| check.account == account) {
        put(&mut check.remote);
        put(&mut check.series);
        put(&mut check.moved);
        if let Some(base) = check.base.as_deref_mut() {
            put(&mut base.origin.id);
            put(&mut base.origin.series);
        }
    }
    let loose: Vec<Id> = mailbox.events.values().filter(|e| e.origin.account == account && e.origin.series == old).map(|e| e.id).collect();
    for id in loose {
        if let Ok(event) = mailbox.event_mut(id) {
            event.origin.series = name.to_string();
        }
    }
}

/// Whether `event` is in a server's calendar and isn't on that server, nor on its way there: one the server refused, or took
/// and doesn't list, or a meeting nobody was invited to yet (in a hosting page's tab; see the module).
pub fn unsent(mailbox: &Mailbox, event: &Event) -> bool {
    remote::keeper(mailbox, &event.origin).is_some()
        && event.origin.id.is_empty()
        && event.origin.series.is_empty()
        && !mailbox.event_changes.iter().any(|change| change.kind == EventChangeKind::Create && change.event == event.id)
}

/// The event `id` stays in its calendar, [`unsent`]: whether it is there, in a calendar a server keeps.
pub(crate) fn hold(mailbox: &mut Mailbox, id: Id) -> bool {
    let Some(account) = mailbox.event(id).and_then(|event| mailbox.calendar(event.calendar)).map(|calendar| calendar.origin.account) else {
        return false;
    };
    if mailbox.account(account).is_none() {
        return false;
    }
    mailbox.event_changes.retain(|change| !(change.kind == EventChangeKind::Create && change.event == id));
    match mailbox.event_mut(id) {
        Ok(event) => {
            event.origin = Origin { account, ..Default::default() };
            true
        }
        Err(_) => false,
    }
}

/// The event `id`, which is [`unsent`], was changed: it goes to the server, but for a meeting, whose people are only written
/// to on the person's yes ([`invite`]).
pub(crate) fn send_again(mailbox: &mut Mailbox, account: Id, id: Id) {
    if mailbox.event(id).is_some_and(|event| event.attendees.is_empty()) {
        invite(mailbox, account, id);
    }
}

/// The event `id` goes to its server when it is [`unsent`], under a mark of its own (the one it went under before may be taken
/// there).
pub(crate) fn invite(mailbox: &mut Mailbox, account: Id, id: Id) {
    let waits = mailbox.event(id).is_some_and(|event| unsent(mailbox, event));
    if !waits {
        return;
    }
    if let Ok(event) = mailbox.event_mut(id) {
        event.uid.clear();
    }
    let token = remote::new_token(id, account);
    mailbox.event_changes.push(EventChange { account, kind: EventChangeKind::Create, event: id, token, ..Default::default() });
}

/// What is in calendars of the mailbox's own goes where the module says, once a server keeps a calendar here; whether
/// anything did. Nothing in a window that keeps what is made in it.
pub fn bring_home(s: &mut Session) -> bool {
    if s.keeps_local {
        return false;
    }
    let Some((main, account)) = remote::server_calendar(&s.mailbox).map(|calendar| (calendar.id, calendar.origin.account)) else { return false };
    let own: Vec<Id> =
        s.mailbox.calendars.iter().filter(|c| remote::keeper(&s.mailbox, &c.origin).is_none() && !c.is_subscribed()).map(|c| c.id).collect();
    if own.is_empty() {
        return false;
    }
    let offset_at = remote::offset_of(s);
    let here: Vec<Id> = s.mailbox.events.values().filter(|event| own.contains(&event.calendar)).map(|event| event.id).collect();
    for id in here {
        let Some(event) = s.mailbox.event(id).cloned() else { continue };
        let uid = event.uid.trim();
        let same = |other: &&std::sync::Arc<Event>| {
            !own.contains(&other.calendar) && !uid.is_empty() && other.uid.trim() == uid && other.origin.series.is_empty()
        };
        let twin = s.mailbox.events.values().find(same).map(|twin| twin.as_ref().clone());
        match twin {
            Some(twin) => double(s, &event, &twin, main, &offset_at),
            None => moved(s, id, main, account),
        }
    }
    let in_use: std::collections::HashSet<Id> = s.mailbox.events.values().map(|event| event.calendar).collect();
    s.mailbox.calendars.retain(|calendar| !own.contains(&calendar.id) || in_use.contains(&calendar.id));
    if s.mailbox.calendar(s.view.in_calendar).is_none() {
        s.view.in_calendar = 0;
    }
    s.mailbox_changed();
    true
}

/// A double of `twin`, an event a server lists: what the person sees of both is the same, and it goes; else what was changed
/// in it is a change of the server's event, or, where that would write to people or the calendar is only read, it stays as an
/// event of its own, unsent.
fn double(s: &mut Session, event: &Event, twin: &Event, main: Id, offset_at: &dyn Fn(Timestamp) -> i32) {
    let shown =
        |e: &Event| (e.title.trim().to_string(), e.location.trim().to_string(), e.description.trim().to_string(), e.all_day, e.recurrence.clone());
    let when = |e: &Event| remote::server_time(e, offset_at);
    if shown(event) == shown(twin) && when(event) == when(twin) {
        let _ = s.mailbox.remove_event(event.id);
        return;
    }
    let quiet = event.attendees.is_empty() && twin.attendees.is_empty() && !twin.origin.guest;
    if quiet && remote::writable(&s.mailbox, twin.calendar).is_ok() && s.mailbox.event(twin.id).is_some() {
        if let Ok(theirs) = s.mailbox.event_mut(twin.id) {
            theirs.title.clone_from(&event.title);
            theirs.location.clone_from(&event.location);
            theirs.description.clone_from(&event.description);
            (theirs.start, theirs.end, theirs.all_day) = (event.start, event.end, event.all_day);
            theirs.start_zone.clone_from(&event.start_zone);
            theirs.end_zone.clone_from(&event.end_zone);
            (theirs.start_repeated, theirs.end_repeated) = (event.start_repeated, event.end_repeated);
        }
        if remote::note_changed(&mut s.mailbox, twin, offset_at).is_ok() {
            let _ = s.mailbox.remove_event(event.id);
            return;
        }
    }
    if let Ok(event) = s.mailbox.event_mut(event.id) {
        event.calendar = main;
        event.uid.clear();
    }
    hold(&mut s.mailbox, event.id);
}

/// An event no server has goes to `main`, the calendar of `account`, and to its server; one with people invited, or that the
/// calendar can't take, waits there, unsent.
fn moved(s: &mut Session, id: Id, main: Id, account: Id) {
    let organizer = s.mailbox.account(account).map(|a| a.address());
    let mut invites = false;
    if let Ok(event) = s.mailbox.event_mut(id) {
        event.calendar = main;
        event.uid.clear();
        invites = !event.attendees.is_empty();
        if invites && event.organizer.is_none() {
            event.organizer = organizer;
        }
    }
    let goes = !invites && remote::writable(&s.mailbox, main).is_ok() && remote::room_for_change(&s.mailbox).is_ok();
    if !goes || remote::note_created(s, id).is_err() {
        hold(&mut s.mailbox, id);
    }
}

#[cfg(test)]
#[path = "hosted_calendar_tests.rs"]
mod tests;
