use std::path::PathBuf;

use once_core::SandboxMode;
use usage::Subcommands;

#[derive(Subcommands)]
pub enum ArtifactCmd {
    /// Build an artifact capability and export a self-contained content-addressed package.
    Export {
        target: String,
        /// Parent directory for immutable packages. Defaults to .once/artifacts.
        #[usage(long)]
        output: Option<PathBuf>,
        #[usage(long, default = "off")]
        sandbox: SandboxMode,
        #[usage(long, value_name = "KEY=VALUE")]
        config: Vec<String>,
    },
    /// Build, export, and explicitly publish through the configured artifact infrastructure.
    Publish {
        target: String,
        /// Named infrastructure override. The local provider retains a self-contained package.
        #[usage(long)]
        provider: Option<String>,
        /// Optional distribution track. Does not change artifact content identity.
        #[usage(long)]
        track: Option<String>,
        #[usage(long, default = "off")]
        sandbox: SandboxMode,
        #[usage(long, value_name = "KEY=VALUE")]
        config: Vec<String>,
    },
}

impl ArtifactCmd {
    pub fn surface_path(&self) -> Vec<&'static str> {
        vec![match self {
            Self::Export { .. } => "export",
            Self::Publish { .. } => "publish",
        }]
    }
}
