mod config;

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
}

fn main() {
  let args = Cli::parse();

  match args.command {
    Commands::Init { name } => {
      let res = config::init_config(name);
      match res {
        Ok(_) => println!("✅ initialized config file"),
        Err(e) => eprintln!("💣 {}", e),
      }
    }
  }
}
