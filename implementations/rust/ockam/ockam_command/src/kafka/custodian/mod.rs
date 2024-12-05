use clap::{command, Args, Subcommand};

use crate::kafka::custodian::create::CreateCommand;
use crate::kafka::custodian::delete::DeleteCommand;
use crate::kafka::custodian::list::ListCommand;
use crate::kafka::custodian::show::ShowCommand;
use crate::{Command, CommandGlobalOpts};

pub(crate) mod create;
pub(crate) mod delete;
pub(crate) mod list;
pub(crate) mod show;

/// Manage Kafka Custodians
#[derive(Clone, Debug, Args)]
#[command(arg_required_else_help = true, subcommand_required = true)]
pub struct KafkaCustodianCommand {
    #[command(subcommand)]
    pub(crate) subcommand: KafkaCustodianSubcommand,
}

#[derive(Clone, Debug, Subcommand)]
pub enum KafkaCustodianSubcommand {
    Create(CreateCommand),
    Show(ShowCommand),
    Delete(DeleteCommand),
    List(ListCommand),
}

impl KafkaCustodianCommand {
    pub fn run(self, opts: CommandGlobalOpts) -> miette::Result<()> {
        match self.subcommand {
            KafkaCustodianSubcommand::Create(c) => c.run(opts),
            KafkaCustodianSubcommand::Show(c) => c.run(opts),
            KafkaCustodianSubcommand::Delete(c) => c.run(opts),
            KafkaCustodianSubcommand::List(c) => c.run(opts),
        }
    }

    pub fn name(&self) -> String {
        match &self.subcommand {
            KafkaCustodianSubcommand::Create(c) => c.name(),
            KafkaCustodianSubcommand::Show(c) => c.name(),
            KafkaCustodianSubcommand::Delete(c) => c.name(),
            KafkaCustodianSubcommand::List(c) => c.name(),
        }
    }
}
