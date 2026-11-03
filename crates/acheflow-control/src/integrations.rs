use acheflow_extension_api::integration::*;
use std::collections::{BTreeMap, VecDeque};

#[derive(Default)]
pub struct Integrations {
    channels: BTreeMap<String, Channel>,
    donations: VecDeque<Donation>,
    interactions: BTreeMap<String, Interaction>,
}
impl Integrations {
    pub fn channels(&self) -> Vec<Channel> {
        self.channels.values().cloned().collect()
    }
    pub fn register_channel(&mut self, origin: &str, mut channel: Channel) -> Result<(), String> {
        if !valid_id(&channel.id, 128)
            || channel.label.is_empty()
            || channel.label.len() > 256
            || channel.capabilities.len() > 8
            || channel.constraints.len() > 16
            || channel.capabilities.iter().any(|c| {
                !matches!(
                    c.as_str(),
                    "chat.send" | "poll.create" | "interaction.triggered"
                )
            })
        {
            return Err("invalid channel descriptor".into());
        }
        channel.extension_id = origin.into();
        let key = channel.key();
        if !self.channels.contains_key(&key) && self.channels.len() >= 64 {
            return Err("channel limit reached".into());
        }
        self.channels.insert(key, channel);
        Ok(())
    }
    pub fn targets(&self, requested: &[String], capability: &str) -> Result<Vec<Channel>, String> {
        let targets: Vec<_> = self
            .channels
            .values()
            .filter(|c| {
                c.capabilities.iter().any(|s| s == capability)
                    && (requested.is_empty() || requested.contains(&c.key()))
            })
            .cloned()
            .collect();
        if targets.is_empty()
            || requested
                .iter()
                .any(|id| !targets.iter().any(|c| c.key() == *id))
        {
            return Err("unavailable: selected channels do not support this action".into());
        }
        Ok(targets)
    }
    pub fn disconnect(&mut self, origin: &str) {
        self.channels.retain(|_, c| c.extension_id != origin);
    }
    pub fn donation(&mut self, origin: &str, mut donation: Donation) -> Result<bool, String> {
        if !valid_id(&donation.id, 256)
            || !valid_id(&donation.channel_id, 128)
            || donation.decimal_places > 6
            || donation.currency.len() != 3
            || !donation.currency.bytes().all(|b| b.is_ascii_uppercase())
            || donation.message.len() > 8192
            || donation.display_name.len() > 256
        {
            return Err("invalid donation".into());
        }
        if self
            .donations
            .iter()
            .any(|d| d.extension_id == origin && d.id == donation.id)
        {
            return Ok(false);
        }
        donation.extension_id = origin.into();
        self.donations.push_back(donation);
        if self.donations.len() > 1000 {
            self.donations.pop_front();
        }
        Ok(true)
    }
    pub fn donations(&self) -> Vec<Donation> {
        self.donations.iter().cloned().collect()
    }
    pub fn register_interaction(
        &mut self,
        origin: &str,
        interaction: Interaction,
    ) -> Result<(), String> {
        self.store_interaction(origin, interaction, true)
    }
    pub fn configure_interaction(
        &mut self,
        origin: &str,
        interaction: Interaction,
    ) -> Result<(), String> {
        self.store_interaction(origin, interaction, false)
    }
    fn store_interaction(
        &mut self,
        origin: &str,
        mut interaction: Interaction,
        preserve_binding: bool,
    ) -> Result<(), String> {
        if !valid_id(&interaction.id, 128)
            || interaction.label.is_empty()
            || interaction.label.len() > 256
            || interaction
                .action
                .as_ref()
                .is_some_and(|a| a.rotation.iter().any(|v| !v.is_finite()))
        {
            return Err("invalid interaction".into());
        }
        interaction.id = format!("{origin}:{}", interaction.id);
        if !self.interactions.contains_key(&interaction.id) && self.interactions.len() >= 128 {
            return Err("interaction limit reached".into());
        }
        if preserve_binding && let Some(old) = self.interactions.get(&interaction.id) {
            if old.action.is_some() {
                interaction.action = old.action.clone();
            }
            interaction.enabled &= old.enabled;
        }
        self.interactions
            .insert(interaction.id.clone(), interaction);
        Ok(())
    }
    pub fn interactions(&self) -> Vec<Interaction> {
        self.interactions.values().cloned().collect()
    }
    pub fn interaction_enabled(&self, origin: &str, id: &str) -> Result<bool, String> {
        self.interactions
            .get(&format!("{origin}:{id}"))
            .map(|i| i.enabled)
            .ok_or_else(|| "interaction not found".into())
    }
    pub fn interaction(&self, origin: &str, id: &str) -> Result<Option<SceneAction>, String> {
        let interaction = self
            .interactions
            .get(&format!("{origin}:{id}"))
            .ok_or("interaction not found")?;
        Ok(interaction
            .enabled
            .then(|| interaction.action.clone())
            .flatten())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channels_are_provider_neutral_and_disabled_sources_disappear() {
        let mut state = Integrations::default();
        for origin in ["one", "two"] {
            state
                .register_channel(
                    origin,
                    Channel {
                        id: "channel".into(),
                        extension_id: "forged".into(),
                        label: origin.into(),
                        capabilities: vec!["chat.send".into()],
                        constraints: Default::default(),
                    },
                )
                .unwrap();
        }
        assert_eq!(state.targets(&[], "chat.send").unwrap().len(), 2);
        assert_eq!(
            state.targets(&["two:channel".into()], "chat.send").unwrap()[0].extension_id,
            "two"
        );
        assert!(state.targets(&[], "poll.create").is_err());
        state.disconnect("two");
        assert!(state.targets(&["two:channel".into()], "chat.send").is_err());
    }
    #[test]
    fn donations_are_deduplicated_per_extension_and_interactions_are_namespaced() {
        let mut state = Integrations::default();
        let donation = Donation {
            id: "receipt".into(),
            extension_id: String::new(),
            channel_id: "channel".into(),
            display_name: "Viewer".into(),
            amount_minor: 125,
            currency: "EUR".into(),
            decimal_places: 2,
            message: String::new(),
            occurred_at_ms: 1,
        };
        state.donation("one", donation.clone()).unwrap();
        state.donation("one", donation.clone()).unwrap();
        state.donation("two", donation).unwrap();
        assert_eq!(state.donations().len(), 2);
        state
            .register_interaction(
                "one",
                Interaction {
                    id: "spin".into(),
                    label: "Spin".into(),
                    action: None,
                    enabled: true,
                },
            )
            .unwrap();
        assert!(state.interaction("two", "spin").is_err());
    }
}
