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
    voters: HashSet<(u64, String, String)>,
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
            ["!vote", option] => {
                if let Ok(option) = option.parse::<usize>() {
                    self.vote(&message, option);
                }
                Ok(None)
            }
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
        });
        if self.messages.len() > MESSAGE_LIMIT {
            self.messages.pop_front();
        }
        true
    }
    fn vote(&mut self, message: &ChatMessage, option: usize) {
        if self.voters.len() >= 100000 {
            return;
        }
        let channel = channel_key(&message.platform, &message.channel_id);
        for poll in self
            .polls
            .iter_mut()
            .filter(|p| !p.closed && p.channels.contains(&channel))
        {
            if !(1..=poll.options.len()).contains(&option) {
                continue;
            }
            if self
                .voters
                .insert((poll.id, message.platform.clone(), message.user_id.clone()))
            {
                poll.votes[option - 1] += 1;
            }
        }
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
            self.messages.len().saturating_sub(limit)
        } else {
            0
        };
        let entries: Vec<_> = self
            .messages
            .iter()
            .filter(|m| m.cursor > after)
            .skip(skip)
            .take(limit)
            .cloned()
            .collect();
        let cursor = entries.last().map(|e| e.cursor).unwrap_or(self.cursor);
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
            let old = self.polls.remove(index).unwrap();
            self.voters.retain(|(id, _, _)| *id != old.id);
        }
        self.next_poll += 1;
        let id = self.next_poll;
        self.polls.push_back(Poll {
            id,
            question: request.question,
            votes: vec![0; request.options.len()],
            options: request.options,
            channels: request.channels,
            closes_at_ms: now.saturating_add(u64::from(request.duration_seconds) * 1000),
            closed: false,
            cancelled: false,
            announcement_error: None,
        });
        Ok(id)
    }

    fn expire_polls(&mut self, now: u64) {
        for poll in &mut self.polls {
            if now >= poll.closes_at_ms {
                poll.closed = true;
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
        poll.closed = true;
        poll.cancelled |= cancel;
        Ok(())
    }
    pub fn announcement_failed(&mut self, id: u64, message: String) {
        if let Some(poll) = self.polls.iter_mut().find(|p| p.id == id) {
            poll.announcement_error = Some(message.chars().take(500).collect());
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
    if !bounded_text(&rule.name, 32) || rule.name == "vote" {
        return Err("invalid or reserved command name".into());
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
        || rule.roles.iter().any(|r| {
            !matches!(
                r.as_str(),
                "broadcaster" | "moderator" | "vip" | "subscriber"
            )
        })
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
    fn votes_are_namespaced_deduplicated_and_close_deterministically() {
        let mut service = Interaction::default();
        let id = service
            .create_poll(
                PollRequest {
                    question: "Where?".into(),
                    options: vec!["Left".into(), "Right".into()],
                    channels: vec!["twitch:a".into()],
                    duration_seconds: 10,
                },
                1000,
            )
            .unwrap();
        service.ingest(chat("1", "u", "!vote 1"), 1001).unwrap();
        service.ingest(chat("1", "u", "!vote 1"), 1002).unwrap();
        service.ingest(chat("2", "u", "!vote 2"), 1003).unwrap();
        service.ingest(chat("3", "v", "!vote 2"), 11000).unwrap();
        assert_eq!(service.polls(11000)[0].votes, [1, 0]);
        assert!(service.polls(11000)[0].closed);
        assert!(service.close_poll(id, false).is_ok());
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
