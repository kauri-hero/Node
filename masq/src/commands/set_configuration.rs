use crate::command_context::CommandContext;
use crate::commands::commands_common::{Command, CommandError};
use clap::{App, Arg, ArgMatches, Error, SubCommand};
use masq_lib::shared_schema::common_validators;
use masq_lib::shared_schema::GAS_PRICE_HELP;
use std::any::Any;
use websocket::server::upgrade::validate;

#[derive(Debug, PartialEq)]
pub struct SetConfigurationCommand {
    pub gas_price_opt: Option<u8>, // shouldn't we lift the limit up? it reached more than 300 Gwei due to busy market
    pub start_block_opt: Option<u64>,
}

impl SetConfigurationCommand {
    pub fn new(pieces: Vec<String>) -> Result<Self, String> {
        match pieces.len() {
            1 => Err("This command is not supported without arguments".to_string()),
            _ => match set_configuration_subcommand().get_matches_from_safe(pieces) {
                Ok(matches) => Ok(SetConfigurationCommand {
                    gas_price_opt: matches.value_of("gas-price").map(|s| {
                        s.to_string().parse::<u8>().expect(
                            "second parsing of the same value: must be a library tool failure ",
                        )
                    }),
                    start_block_opt: matches.value_of("start-block").map(|s| {
                        s.to_string().parse::<u64>().expect(
                            "second parsing of the same value: must be a library tool failure ",
                        )
                    }),
                }),
                Err(e) => Err(format!("{}", e)),
            },
        }
    }
}

fn validate_start_block(start_block: String) -> Result<(), String> {
    match start_block.parse::<u64>() {
        Ok(_) => Ok(()), //how to write a correct check for the valid range of this number  if any
        _ => Err(start_block),
    }
}

impl Command for SetConfigurationCommand {
    fn execute(&self, context: &mut dyn CommandContext) -> Result<(), CommandError> {
        unimplemented!()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn set_configuration_subcommand() -> App<'static, 'static> {
    SubCommand::with_name("set-configuration")
        .about("Sets Node configuration values being enabled for that operation when the Node is running")
        .arg(
            Arg::with_name("gas-price")
                .help(&GAS_PRICE_HELP)
                .long("gas-price")
                .value_name("GAS-PRICE")
                .required(false)
                .validator(common_validators::validate_gas_price)

        )
        .arg(
            Arg::with_name("start-block")
                .help("Order number of the Ethereum block where scanning for your personal transaction should start at.\
                Be careful not to choose a number higher than the historically biggest one on the blockchain.")  // change the narrative TODO: you should maybe create an interactive help reflecting values in the config table
                .long("start-block")
                .value_name("START-BLOCK")
                .required(false)
                .validator(validate_start_block)
        )
}
