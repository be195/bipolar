mod config;
mod build;
mod utils;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "bipolar")]
#[command(about = "tool for a/b testing", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Init {
        #[arg(short, long)]
        name: Option<String>,
    },
    FooBar,
    Build {
        #[arg()]
        nuclear: Option<bool>,
    }
}

fn main() {
    let args = Cli::parse();

    match args.command {
        Commands::Init { name } => {
            if let Err(e) = config::init_config(name) {
                eprintln!("error initializing config: {}", e);
                std::process::exit(1);
            }
        },
        Commands::FooBar => {
            let config = config::try_load_config();
            println!("config loaded: {:?}", config);
        },
        Commands::Build { nuclear } => {
            let config = config::try_load_config();
            if let Err(e) = build::build(&config, nuclear) {
                eprintln!("error building: {}", e);
                std::process::exit(1);
            }
        },
    }
}
