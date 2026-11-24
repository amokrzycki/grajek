use anyhow::{Context as _, Result};

use super::Live;
use crate::discord::{Discord, Health, setup};

impl Live {
    pub(super) async fn restore(
        &self,
        config: crate::config::Config,
        saved: Option<setup::Destination>,
    ) -> Result<()> {
        use setup::{Credential, Request, credential};
        let token = match config.token {
            Some(token) => Some(token),
            None => match credential(Credential::Read).await {
                Ok(token) => token,
                Err(message) => {
                    self.health
                        .send_modify(|health| health.setup.storage_notice = Some(message.into()));
                    None
                }
            },
        };
        let Some(token) = token else {
            return Ok(());
        };
        self.configure(Request::Authenticate {
            token,
            remember: false,
        })
        .await?;
        self.configure(Request::Refresh).await?;
        let bot = self
            .health
            .borrow()
            .setup
            .bot
            .context("Bot not authenticated.")?;
        let destination = if let Some((guild, channel)) = config.destination {
            Some(setup::Destination {
                bot,
                guild,
                channel,
                guild_name: String::new(),
                channel_name: String::new(),
            })
        } else {
            saved.filter(|destination| destination.bot == bot)
        };
        if let Some(destination) = destination {
            self.configure(Request::Select(destination)).await?;
        }
        Ok(())
    }

    pub(super) async fn refresh(&self, discord: &Discord) -> Result<()> {
        let epoch = self.health.borrow().discovery_epoch;
        let mut servers = Vec::new();
        let mut after = None;
        loop {
            let page = discord.guilds(after).await?;
            let count = page.len();
            after = page.last().map(|server| server.id);
            servers.extend(page);
            if count < 200 {
                break;
            }
        }
        let mut destination_lost = false;
        self.health.send_modify(|health| {
            destination_lost = health.refresh_servers(epoch, servers);
        });
        if destination_lost {
            discord.disconnect().await?;
        }
        Ok(())
    }

    pub(super) async fn configure(&self, request: setup::Request) -> Result<()> {
        use setup::{Credential, Request, Status, credential};
        match request {
            Request::Retry | Request::Restore { .. } => {
                anyhow::bail!("Restore must use the setup entry point")
            }
            Request::Authenticate { token, remember } => {
                let mut session = self.discord.write().await;
                *self.credentials.lock().await = Some((token.clone(), remember));
                if let Some(discord) = session.take() {
                    discord.shutdown().await;
                }
                self.health.send_modify(|health| {
                    let session_epoch = health.session_epoch.wrapping_add(1);
                    let voice_epoch = health.voice_epoch.wrapping_add(1);
                    *health = Health {
                        session_epoch,
                        voice_epoch,
                        ..Health::default()
                    };
                    health.setup.external = self.external;
                    health.setup.status = Status::Checking;
                });
                let discord = Discord::start(token.trim(), self.health.clone()).await?;
                if remember
                    && !self.external
                    && let Err(message) = credential(Credential::Save(token)).await
                {
                    discord.shutdown().await;
                    self.health.send_modify(|health| {
                        health.gateway = false;
                        health.setup.bot = None;
                        health.setup.storage_notice = Some(message.into());
                    });
                    anyhow::bail!(message);
                }
                *session = Some(discord);
            }
            Request::Remove => {
                let mut session = self.discord.write().await;
                anyhow::ensure!(
                    !self.external,
                    "This token is externally managed. Remove it from your environment to change it."
                );
                credential(Credential::Remove)
                    .await
                    .map_err(anyhow::Error::msg)?;
                if let Some(discord) = session.take() {
                    discord.shutdown().await;
                }
                drop(session);
                *self.credentials.lock().await = None;
                self.health.send_modify(|health| {
                    *health = Health {
                        session_epoch: health.session_epoch.wrapping_add(1),
                        voice_epoch: health.voice_epoch.wrapping_add(1),
                        ..Health::default()
                    };
                });
            }
            Request::Refresh => {
                let session = self.discord.read().await;
                self.refresh(session.as_ref().context("Configure a bot first.")?)
                    .await?;
                drop(session);
            }
            Request::Channels(guild) => {
                let session = self.discord.read().await;
                let discord = session.as_ref().context("Configure a bot first.")?;
                self.health.send_modify(|health| {
                    health.setup.browsing = Some(guild);
                    health.setup.channels.clear();
                });
                let epoch = self.health.borrow().discovery_epoch;
                let channels = discord.channels(guild).await?;
                drop(session);
                self.health.send_modify(|health| {
                    if health.discovery_epoch == epoch {
                        health.setup.channels = channels;
                    }
                });
            }
            Request::Select(destination) => {
                let mut session = self.discord.write().await;
                session
                    .as_mut()
                    .context("Configure a bot first.")?
                    .select(destination)
                    .await?;
                drop(session);
            }
        }
        Ok(())
    }
}
