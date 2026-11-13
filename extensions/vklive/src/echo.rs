//! VK returns no message ID on send. Match one confirmed echo conservatively.
use std::collections::VecDeque;

pub(super) struct Outgoing {
    pub request: String,
    pub text: String,
    pub stream: String,
    pub started: u64,
}
struct Confirmed {
    outgoing: Outgoing,
    expires: u64,
}
#[derive(Default)]
pub(super) struct Echoes {
    user: u64,
    confirmed: VecDeque<Confirmed>,
}
impl Echoes {
    pub fn set_user(&mut self, user: u64) {
        if self.user != user {
            self.confirmed.clear();
        }
        self.user = user;
    }
    pub fn confirm(&mut self, outgoing: Outgoing, now: u64) {
        self.confirmed.retain(|sent| sent.expires >= now);
        if self.confirmed.len() >= 128 {
            self.confirmed.pop_front();
        }
        self.confirmed.push_back(Confirmed {
            outgoing,
            expires: now.saturating_add(30_000),
        });
    }
    pub fn suppress(
        &mut self,
        author: u64,
        text: &str,
        created: u64,
        stream: &str,
        now: u64,
    ) -> bool {
        self.confirmed.retain(|sent| sent.expires >= now);
        if self.user == 0 || author != self.user {
            return false;
        }
        let found = self.confirmed.iter().position(|sent| {
            sent.outgoing.stream == stream
                && sent.outgoing.text == text
                && created >= sent.outgoing.started / 1000 * 1000
                && created <= sent.expires
        });
        if let Some(index) = found {
            self.confirmed.remove(index);
            return true;
        }
        false
    }
}
