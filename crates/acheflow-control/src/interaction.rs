use acheflow_extension_api::interaction::*;
use std::collections::{BTreeMap, HashSet, VecDeque};

const MESSAGE_LIMIT: usize = 2000;
type MessageKey = (String, String, String);

#[derive(Default)]
pub struct Interaction {
    messages: VecDeque<ChatEntry>,
    seen: HashSet<MessageKey>,
    seen_order: VecDeque<MessageKey>,
    cursor: u64,
    polls: VecDeque<Poll>,
    next_poll: u64,
    commands: BTreeMap<(String, String), CommandRule>,
    cooldowns: BTreeMap<(String, String), u64>,
}

impl Interaction {
    pub fn cursor(&self) -> u64 {
        self.cursor
    }
    pub fn ingest(
        &mut self,
        message: ChatMessage,
        now: u64,
    ) -> Result<Option<CommandRule>, String> {
        validate_message(&message)?;
        if !self.remember(&message) {
            return Ok(None);
        }
        self.expire_polls(now);
        let words: Vec<_> = message.text.split_whitespace().take(3).collect();
        match words.as_slice() {
            [command] => Ok(command
                .strip_prefix('!')
                .and_then(|name| self.chat_command(&message, name, now))),
            _ => Ok(None),
        }
    }
    fn remember(&mut self, message: &ChatMessage) -> bool {
        let key = (
            message.platform.clone(),
            message.channel_id.clone(),
            message.message_id.clone(),
        );
        if !self.seen.insert(key.clone()) {
            return false;
        }
        self.seen_order.push_back(key);
        if self.seen_order.len() > MESSAGE_LIMIT * 4
            && let Some(old) = self.seen_order.pop_front()
        {
            self.seen.remove(&old);
        }
        self.cursor += 1;
        self.messages.push_back(ChatEntry {
            cursor: self.cursor,
            message: message.clone(),
            deleted: false,
            merged_into: None,
            delivery: vec![],
        });
        if self.messages.len() > MESSAGE_LIMIT {
            self.messages.pop_front();
        }
        true
    }
    fn chat_command(&mut self, message: &ChatMessage, name: &str, now: u64) -> Option<CommandRule> {
        let key = (
            channel_key(&message.platform, &message.channel_id),
            name.to_owned(),
        );
        let rule = self.commands.get(&key)?;
        if !rule.enabled {
            return None;
        }
        if !rule.roles.is_empty() && !message.roles.iter().any(|r| rule.roles.contains(r)) {
            return None;
        }
        if self
            .cooldowns
            .get(&key)
            .is_some_and(|at| now.saturating_sub(*at) < rule.cooldown_ms)
        {
            return None;
        }
        self.cooldowns.insert(key, now);
        Some(rule.clone())
    }

    pub fn messages(&self, after: u64, limit: usize) -> ChatPage {
        let limit = limit.clamp(1, 200);
        let skip = if after == 0 {
            self.messages
                .iter()
                .filter(|e| e.merged_into.is_none())
                .count()
                .saturating_sub(limit)
        } else {
            0
        };
        let entries: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.cursor > after && (after > 0 || m.merged_into.is_none()))
            .skip(skip)
            .take(limit)
            .cloned()
            .collect();
        let cursor = if after == 0 {
            self.cursor
        } else {
            entries.last().map(|e| e.cursor).unwrap_or(self.cursor)
        };
        ChatPage {
            entries,
            cursor,
            gap: after > self.cursor
                || self
                    .messages
                    .front()
                    .is_some_and(|e| after > 0 && after < e.cursor.saturating_sub(1)),
        }
    }

    pub fn delete(&mut self, platform: &str, channel: &str, id: Option<&str>) {
        let mut deleted = vec![];
        for entry in &mut self.messages {
            if entry.message.platform == platform
                && entry.message.channel_id == channel
                && id.is_none_or(|id| entry.message.message_id == id)
            {
                entry.deleted = true;
                entry.message.text.clear();
                deleted.push(entry.clone());
            }
        }
        self.messages
            .retain(|entry| !deleted.iter().any(|d| d.cursor == entry.cursor));
        for mut entry in deleted {
            self.cursor += 1;
            entry.cursor = self.cursor;
            self.messages.push_back(entry);
        }
    }

    pub fn create_poll(&mut self, request: PollRequest, now: u64) -> Result<u64, String> {
        self.expire_polls(now);
        validate_poll(&request)?;
        if self
            .polls
            .iter()
            .any(|p| !p.closed && p.channels.iter().any(|c| request.channels.contains(c)))
        {
            return Err("a poll is already active in this channel".into());
        }
        if self.polls.len() >= 100 {
            let index = self
                .polls
                .iter()
                .position(|p| p.closed)
                .ok_or("poll limit reached")?;
            self.polls.remove(index);
        }
        self.next_poll += 1;
        let id = self.next_poll;
        self.polls.push_back(Poll {
            id,
            question: request.question,
            votes: vec![0; request.options.len()],
            options: request.options.clone(),
            channels: request.channels.clone(),
            closes_at_ms: now.saturating_add(u64::from(request.duration_seconds) * 1000),
            closed: false,
            cancelled: false,
            targets: request
                .channels
                .iter()
                .map(|channel| acheflow_extension_api::integration::PollTarget {
                    channel: channel.clone(),
                    external_id: None,
                    state: "pending".into(),
                    votes: vec![0; request.options.len()],
                    error: None,
                    updated_at_ms: now,
                })
                .collect(),
        });
        Ok(id)
    }

    fn expire_polls(&mut self, now: u64) {
        for poll in &mut self.polls {
            if now >= poll.closes_at_ms {
                for target in &mut poll.targets {
                    if !matches!(target.state.as_str(), "completed" | "failed")
                        && now > poll.closes_at_ms.saturating_add(30000)
                    {
                        target.error = Some("Final poll result was not received".into());
                    }
                }
                poll.closed = poll
                    .targets
                    .iter()
                    .all(|t| matches!(t.state.as_str(), "completed" | "failed"));
            }
        }
    }
    pub fn polls(&mut self, now: u64) -> Vec<Poll> {
        self.expire_polls(now);
        self.polls.iter().cloned().collect()
    }
    pub fn close_poll(&mut self, id: u64, cancel: bool) -> Result<(), String> {
        let poll = self
            .polls
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("poll not found")?;
        for target in &mut poll.targets {
            if !matches!(target.state.as_str(), "completed" | "failed") {
                target.state = "closing".into();
            }
        }
        poll.cancelled |= cancel;
        Ok(())
    }
    pub fn update_poll(
        &mut self,
        origin: &str,
        update: acheflow_extension_api::integration::PollUpdate,
        now: u64,
    ) -> Result<(), String> {
        let poll = self
            .polls
            .iter_mut()
            .find(|p| p.id == update.poll_id)
            .ok_or("poll not found")?;
        if update.votes.len() != poll.options.len()
            || !matches!(update.state.as_str(), "active" | "completed" | "failed")
            || update.error.as_ref().is_some_and(|e| e.len() > 500)
            || update.external_id.as_ref().is_some_and(|id| id.len() > 256)
        {
            return Err("invalid poll update".into());
        }
        let key = channel_key(origin, &update.channel_id);
        let target = poll
            .targets
            .iter_mut()
            .find(|t| t.channel == key)
            .ok_or("forbidden: poll channel is not targeted")?;
        if matches!(target.state.as_str(), "completed" | "failed") {
            return Ok(());
        }
        if target.external_id.is_some() && update.external_id != target.external_id {
            return Err("conflict: external poll identity changed".into());
        }
        target.external_id = update.external_id;
        if target.state != "closing" || update.state != "active" {
            target.state = update.state;
        }
        target.votes = update.votes;
        target.error = update.error;
        target.updated_at_ms = now;
        poll.votes = vec![0; poll.options.len()];
        for target in &poll.targets {
            for (sum, vote) in poll.votes.iter_mut().zip(&target.votes) {
                *sum = sum.saturating_add(*vote);
            }
        }
        poll.closed = poll
            .targets
            .iter()
            .all(|t| matches!(t.state.as_str(), "completed" | "failed"));
        Ok(())
    }
    pub fn begin_send(
        &mut self,
        id: &str,
        text: &str,
        channels: &[String],
        now: u64,
    ) -> Result<(), String> {
        if text.trim().is_empty() || text.len() > 8192 {
            return Err("invalid chat text".into());
        }
        self.remember(&ChatMessage {
            platform: "acheflow".into(),
            channel_id: "outgoing".into(),
            message_id: id.into(),
            user_id: "local".into(),
            display_name: "You".into(),
            text: text.into(),
            roles: vec![],
            occurred_at_ms: now,
        });
        if let Some(entry) = self.messages.back_mut() {
            entry.delivery = channels
                .iter()
                .map(|channel| acheflow_extension_api::integration::Delivery {
                    channel: channel.clone(),
                    state: "pending".into(),
                    error: None,
                    external_id: None,
                })
                .collect();
        }
        Ok(())
    }
    pub fn delivery(
        &mut self,
        origin: &str,
        channel: &str,
        id: &str,
        sent: Option<bool>,
        external_id: Option<String>,
        error: Option<String>,
    ) -> Result<(), String> {
        if error.as_ref().is_some_and(|e| e.len() > 500)
            || external_id.as_ref().is_some_and(|e| e.len() > 256)
        {
            return Err("invalid delivery result".into());
        }
        let entry = self
            .messages
            .iter_mut()
            .find(|e| e.message.platform == "acheflow" && e.message.message_id == id)
            .ok_or("send operation not found")?;
        let key = channel_key(origin, channel);
        let target = entry
            .delivery
            .iter_mut()
            .find(|t| t.channel == key)
            .ok_or("forbidden: delivery channel is not targeted")?;
        if target.state != "pending" {
            return Ok(());
        }
        target.state = match sent {
            Some(true) => "sent",
            Some(false) => "failed",
            None => "unknown",
        }
        .into();
        target.error = error;
        target.external_id = external_id.clone();
        let updated = entry.clone();
        self.republish(updated);
        if let Some(external_id) = external_id {
            let message = ChatMessage {
                platform: origin.into(),
                channel_id: channel.into(),
                message_id: external_id.clone(),
                user_id: "receipt".into(),
                display_name: String::new(),
                text: String::new(),
                roles: vec![],
                occurred_at_ms: 0,
            };
            let echoes: Vec<_> = self
                .messages
                .iter()
                .filter(|e| {
                    e.message.platform == origin
                        && e.message.channel_id == channel
                        && e.message.message_id == external_id
                        && e.merged_into.is_none()
                })
                .cloned()
                .collect();
            for mut echo in echoes {
                echo.deleted = true;
                echo.message.text.clear();
                echo.merged_into = Some(id.into());
                self.republish(echo);
            }
            // Remember without emitting a second history entry, including echoes arriving after the receipt.
            let key = (message.platform, message.channel_id, message.message_id);
            if self.seen.insert(key.clone()) {
                self.seen_order.push_back(key);
            }
            while self.seen_order.len() > MESSAGE_LIMIT * 4 {
                if let Some(old) = self.seen_order.pop_front() {
                    self.seen.remove(&old);
                }
            }
        }
        Ok(())
    }
    fn republish(&mut self, mut entry: ChatEntry) {
        self.messages.retain(|old| old.cursor != entry.cursor);
        self.cursor += 1;
        entry.cursor = self.cursor;
        self.messages.push_back(entry);
        if self.messages.len() > MESSAGE_LIMIT {
            self.messages.pop_front();
        }
    }
    pub fn disconnect(&mut self, origin: &str) {
        let prefix = format!("{origin}:");
        let mut updates = vec![];
        for entry in &mut self.messages {
            let mut changed = false;
            for target in &mut entry.delivery {
                if target.channel.starts_with(&prefix) && target.state == "pending" {
                    target.state = "unknown".into();
                    target.error = Some("Extension stopped before delivery was confirmed".into());
                    changed = true;
                }
            }
            if changed {
                updates.push(entry.clone());
            }
        }
        for entry in updates {
            self.republish(entry);
        }
        for poll in &mut self.polls {
            for target in &mut poll.targets {
                if target.channel.starts_with(&prefix)
                    && !matches!(target.state.as_str(), "completed" | "failed")
                {
                    target.state = "failed".into();
                    target.error = Some("Extension stopped before the final poll result".into());
                }
            }
            poll.closed = poll
                .targets
                .iter()
                .all(|t| matches!(t.state.as_str(), "completed" | "failed"));
        }
    }
    pub fn commands(&self) -> Vec<CommandRule> {
        self.commands.values().cloned().collect()
    }
    pub fn set_command(&mut self, rule: CommandRule) -> Result<(), String> {
        validate_command(&rule)?;
        let key = (rule.channel.clone(), rule.name.clone());
        if !self.commands.contains_key(&key) && self.commands.len() >= 128 {
            return Err("command limit reached".into());
        }
        self.commands.insert(key, rule);
        Ok(())
    }
}

fn bounded_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max
}
fn validate_message(message: &ChatMessage) -> Result<(), String> {
    for (value, max) in [
        (&message.platform, 64),
        (&message.channel_id, 128),
        (&message.message_id, 256),
        (&message.user_id, 128),
    ] {
        if !bounded_text(value, max) {
            return Err("invalid chat identity".into());
        }
    }
    if message.text.len() > 8192 || message.display_name.len() > 256 {
        return Err("chat text exceeds its limit".into());
    }
    if message.roles.len() > 32 || message.roles.iter().any(|r| r.len() > 64) {
        return Err("invalid chat roles".into());
    }
    Ok(())
}
fn validate_poll(request: &PollRequest) -> Result<(), String> {
    if !bounded_text(&request.question, 500) {
        return Err("invalid poll question".into());
    }
    if !(2..=10).contains(&request.options.len())
        || request.options.iter().any(|s| !bounded_text(s, 200))
    {
        return Err("poll requires 2–10 nonempty options".into());
    }
    if !(1..=64).contains(&request.channels.len()) {
        return Err("invalid poll channel count".into());
    }
    if request
        .channels
        .iter()
        .any(|s| s.len() > 200 || !s.contains(':'))
    {
        return Err("invalid poll channel".into());
    }
    if !(10..=86400).contains(&request.duration_seconds) {
        return Err("poll duration must be 10–86400 seconds".into());
    }
    Ok(())
}
fn validate_command(rule: &CommandRule) -> Result<(), String> {
    if !bounded_text(&rule.name, 32) {
        return Err("invalid command name".into());
    }
    if !rule
        .name
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err("command names must be alphanumeric".into());
    }
    if rule.channel.len() > 200 || !rule.channel.contains(':') {
        return Err("invalid command channel".into());
    }
    if rule.roles.len() > 16
        || rule
            .roles
            .iter()
            .any(|r| !acheflow_extension_api::integration::valid_id(r, 64))
    {
        return Err("invalid command roles".into());
    }
    if rule.rotation.iter().any(|n| !n.is_finite()) {
        return Err("invalid command rotation".into());
    }
    if rule.cooldown_ms > 86400000 {
        return Err("command cooldown exceeds one day".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn chat(id: &str, user: &str, text: &str) -> ChatMessage {
        ChatMessage {
            platform: "twitch".into(),
            channel_id: "a".into(),
            message_id: id.into(),
            user_id: user.into(),
            display_name: user.into(),
            text: text.into(),
            roles: vec![],
            occurred_at_ms: 1000,
        }
    }
    #[test]
    fn poll_votes_come_from_providers_and_close_waits_for_final_results() {
        let mut service = Interaction::default();
        let id = service
            .create_poll(
                PollRequest {
                    question: "Where?".into(),
                    options: vec!["Left".into(), "Right".into()],
                    channels: vec!["first:a".into(), "second:b".into()],
                    duration_seconds: 10,
                },
                1000,
            )
            .unwrap();
        service.ingest(chat("1", "u", "!vote 1"), 1001).unwrap();
        assert_eq!(service.polls(1001)[0].votes, [0, 0]);
        service
            .update_poll(
                "first",
                acheflow_extension_api::integration::PollUpdate {
                    poll_id: id,
                    channel_id: "a".into(),
                    external_id: Some("external".into()),
                    state: "active".into(),
                    votes: vec![3, 2],
                    error: None,
                },
                1002,
            )
            .unwrap();
        service.close_poll(id, false).unwrap();
        assert!(!service.polls(1003)[0].closed);
        for (origin, channel, votes) in [("first", "a", vec![4, 3]), ("second", "b", vec![1, 2])] {
            service
                .update_poll(
                    origin,
                    acheflow_extension_api::integration::PollUpdate {
                        poll_id: id,
                        channel_id: channel.into(),
                        external_id: Some("external".into()),
                        state: "completed".into(),
                        votes,
                        error: None,
                    },
                    1004,
                )
                .unwrap();
        }
        assert_eq!(service.polls(1005)[0].votes, [5, 5]);
        assert!(service.polls(1005)[0].closed);
    }
    #[test]
    fn cursor_reads_receive_delivery_changes_and_echo_merge_tombstones() {
        let mut service = Interaction::default();
        service
            .begin_send("intent", "hello", &["twitch:a".into()], 1000)
            .unwrap();
        let cursor = service.cursor();
        service
            .ingest(chat("external", "account", "hello"), 1001)
            .unwrap();
        let echo_cursor = service.cursor();
        service
            .delivery(
                "twitch",
                "a",
                "intent",
                Some(true),
                Some("external".into()),
                None,
            )
            .unwrap();
        let delta = service.messages(echo_cursor, 200);
        assert!(delta.cursor > echo_cursor);
        assert!(
            delta
                .entries
                .iter()
                .any(|e| e.message.message_id == "intent" && e.delivery[0].state == "sent")
        );
        assert!(
            delta
                .entries
                .iter()
                .any(|e| e.message.message_id == "external"
                    && e.merged_into.as_deref() == Some("intent"))
        );
        assert_eq!(service.messages(0, 200).entries.len(), 1);
        service
            .begin_send("pending", "another", &["twitch:a".into()], 1002)
            .unwrap();
        let before = service.cursor();
        service.disconnect("twitch");
        assert!(
            service
                .messages(before, 200)
                .entries
                .iter()
                .any(|e| e.delivery.iter().any(|d| d.state == "unknown"))
        );
        assert!(service.cursor() > cursor);
    }
    #[test]
    fn history_has_explicit_gaps_and_deleted_messages_are_scrubbed() {
        let mut service = Interaction::default();
        for id in 0..2010 {
            service
                .ingest(chat(&id.to_string(), "u", "hello"), 1000)
                .unwrap();
        }
        let page = service.messages(1, 100);
        assert!(page.gap);
        assert_eq!(page.entries.len(), 100);
        service.delete("twitch", "a", Some("2009"));
        let last = service.messages(2000, 100);
        assert!(last.entries.last().unwrap().deleted);
        assert!(last.entries.last().unwrap().message.text.is_empty());
    }
    #[test]
    fn command_roles_and_cooldowns_cannot_be_bypassed_by_duplicate_events() {
        let mut service = Interaction::default();
        service
            .set_command(CommandRule {
                name: "spin".into(),
                channel: "twitch:a".into(),
                roles: vec!["moderator".into()],
                cooldown_ms: 1000,
                scene_id: 1,
                object_id: 1,
                rotation: [0., 45., 0.],
                enabled: true,
            })
            .unwrap();
        assert!(
            service
                .ingest(chat("1", "u", "!spin"), 1000)
                .unwrap()
                .is_none()
        );
        let mut message = chat("2", "m", "!spin");
        message.roles.push("moderator".into());
        assert!(service.ingest(message.clone(), 1000).unwrap().is_some());
        assert!(service.ingest(message, 2001).unwrap().is_none());
        let mut next = chat("3", "m", "!spin");
        next.roles.push("moderator".into());
        assert!(service.ingest(next, 1500).unwrap().is_none());
    }
}
