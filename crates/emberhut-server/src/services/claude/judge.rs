//! The ask guard's judge model through the agent CLI: one turn with no tools and no MCP
//! servers, its instructions after the CLI's own system prompt, its answer's text out. Run on
//! the first configured account; given a minute.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;

use super::cli::{CliSpawner, Launch};
use super::stream::{self, Said};
use crate::services::guard::Judge;

/// How long the judge may take.
const TIMEOUT: Duration = Duration::from_mins(1);

/// The judge, as one CLI turn.
#[derive(Clone, Debug)]
pub struct CliJudge {
    spawner: Arc<dyn CliSpawner>,
    command: PathBuf,
    model: Option<String>,
    config_dir: Option<PathBuf>,
}

impl CliJudge {
    /// A judge running `command` (on `model`, with the CLI configuration `config_dir`) through
    /// `spawner`.
    #[must_use]
    pub fn new(
        spawner: Arc<dyn CliSpawner>,
        command: PathBuf,
        model: Option<String>,
        config_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            spawner,
            command,
            model,
            config_dir,
        }
    }

    fn launch(&self) -> Launch {
        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--strict-mcp-config",
            "--permission-mode=dontAsk",
            "--max-turns=1",
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(model) = &self.model {
            args.push(format!("--model={model}"));
        }
        let mut env = Vec::new();
        if let Some(dir) = &self.config_dir {
            env.push(("CLAUDE_CONFIG_DIR".to_owned(), dir.display().to_string()));
        }
        Launch {
            program: self.command.clone(),
            args,
            env,
            env_remove: vec!["CLAUDECODE".to_owned()],
            cwd: None,
        }
    }
}

impl Judge for CliJudge {
    fn judge<'a>(&'a self, rules: &'a str, prompt: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let mut pipes = match self.spawner.spawn(&self.launch()) {
                Ok(pipes) => pipes,
                Err(error) => {
                    tracing::warn!(%error, "the ask guard's judge didn't start");
                    return None;
                }
            };
            if !pipes.send(stream::initialize("judge-1", rules))
                || !pipes.send(stream::user_message(prompt))
            {
                return None;
            }
            let answer = tokio::time::timeout(TIMEOUT, async {
                while let Some(line) = pipes.output.recv().await {
                    match Said::read(&line) {
                        Said::Result { is_error, text, .. } => {
                            return (!is_error).then_some(text);
                        }
                        Said::ControlRequest { id, subtype }
                            if !pipes.send(stream::unsupported(&id, &subtype)) =>
                        {
                            return None;
                        }
                        _ => {}
                    }
                }
                None
            })
            .await
            .ok()
            .flatten();
            pipes.close().await;
            answer
        })
    }
}
