mod config;
mod deinflect;
mod dict;

extern crate clap;

use anyhow::{bail, Result};
use clap::{App, Arg, SubCommand};
use config::Config;
use dict::{lookup, DictConn, DictDb};
use fs::read_to_string;
use genanki_rs::{Error, Note};
use std::{fs, path::Path};

fn main() -> Result<()> {
    let matches = App::new("ace")
        .version("1.0")
        .author("Kamui")
        .about("Anime card exporter for Anki")
        .arg(
            Arg::with_name("config")
                .short("c")
                .long("config")
                .value_name("FILE")
                .help("Sets a custom config file")
                .takes_value(true),
        )
        .subcommand(
            SubCommand::with_name("import")
                .about("Import a dictionary into the database")
                .arg(
                    Arg::with_name("DIR")
                        .required(true)
                        .help("print debug information verbosely")
                        .index(1),
                ),
        )
        .get_matches();

    // Load configuration
    // let config = matches.value_of("config").unwrap_or("config.sample.toml");
    // let conf_text = fs::read_to_string(config).unwrap();
    // let config: Config = toml::from_str(&conf_text).unwrap();

    let mut dictdb = DictDb::new()?;

    if let Some(import_matches) = matches.subcommand_matches("import") {
        let dict_path = import_matches.value_of("DIR").unwrap();
        dictdb.load_yomichan_dict(Path::new(dict_path))?;
    }

    Ok(())
}
