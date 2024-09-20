use std::{
    path::PathBuf,
    process::{self, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use anyhow::{anyhow, bail, Context};
use fs_err as fs;
use fs_err::File;
use roblox_install::RobloxStudio;

use crate::{
    message_receiver::{Message, MessageReceiver, MessageReceiverOptions, RobloxMessage},
    plugin::RunInRbxPlugin,
};

/// A wrapper for process::Child that force-kills the process on drop.
struct KillOnDrop(process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ignored = self.0.kill();
    }
}

pub struct PlaceRunner {
    pub port: u16,
    pub place_path: PathBuf,
    pub server_id: String,
    pub lua_script: String,
    pub debug: bool,
}

impl PlaceRunner {
    fn get_studio_execution_cmd(&self, studio_install: &RobloxStudio) -> String {
        let app_path = studio_install
            .application_path()
            .to_str()
            .unwrap()
            .to_string();

        if app_path.contains("vinegar") {
            "vinegar".to_string()
        } else {
            app_path
        }
    }

    pub fn run(&self, sender: mpsc::Sender<Option<RobloxMessage>>) -> Result<(), anyhow::Error> {
        let studio_install =
            RobloxStudio::locate().context("Could not locate a Roblox Studio installation.")?;

        let plugin_file_path = studio_install
            .plugins_path()
            .join(format!("run_in_roblox-{}.rbxmx", self.port));

        let plugin = RunInRbxPlugin {
            port: self.port,
            server_id: &self.server_id,
            lua_script: &self.lua_script,
        };

        let plugin_file = File::create(&plugin_file_path)?;
        plugin.write(plugin_file)?;

        let message_receiver = MessageReceiver::start(MessageReceiverOptions {
            port: self.port,
            server_id: self.server_id.to_owned(),
        });

        let mut studio_cmd = Command::new(self.get_studio_execution_cmd(&studio_install));

        if studio_cmd
            .get_program()
            .to_str()
            .unwrap()
            .contains("vinegar")
        {
            studio_cmd.arg("studio");
            studio_cmd.arg("run");
        }

        studio_cmd.arg(format!("{}", self.place_path.display()));

        if !self.debug {
            studio_cmd.stdout(Stdio::null());
            studio_cmd.stderr(Stdio::null());
        }

        let _studio_process = KillOnDrop(
            studio_cmd
                .spawn()?,
        );

        let first_message = message_receiver
            .recv_timeout(Duration::from_secs(60))
            .ok_or_else(|| {
                anyhow!("Timeout reached while waiting for Roblox Studio to come online")
            })?;

        match first_message {
            Message::Start => {}
            _ => bail!("Invalid first message received from Roblox Studio plugin"),
        }

        loop {
            match message_receiver.recv() {
                Message::Start => {}
                Message::Stop => {
                    sender.send(None)?;
                    break;
                }
                Message::Messages(roblox_messages) => {
                    for message in roblox_messages.into_iter() {
                        sender.send(Some(message))?;
                    }
                }
            }
        }

        message_receiver.stop();
        fs::remove_file(&plugin_file_path)?;

        Ok(())
    }
}
