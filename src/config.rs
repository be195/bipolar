use serde::{Serialize, Deserialize};
use std::{fs::File, io::{Read, Write}};
use git2::Repository;

#[derive(Debug, Deserialize, Serialize)]
pub struct ExperimentConfig {
  pub name: String,
  pub repo: String,
  pub base: String,
  pub treatments: Vec<Treatment>,
  pub trigger: Trigger,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Treatment {
  pub name: String,
  pub ref_field: Option<String>,
  pub commit: Option<String>,
  pub patch: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Trigger {
  #[serde(rename="hash_mod")]
  HashMod {
    key: String,
    modulus: u32,
    threshold: u32,
  }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag="type")]
pub enum Config {
  #[serde(rename="experiment")]
  Experiment(ExperimentConfig),
}

const CONFIG_FILE: &str = "bipolar.toml";

pub fn load_config() -> Result<ExperimentConfig, Box<dyn std::error::Error>> {
  let mut file = File::open(CONFIG_FILE)?;

  let mut contents = String::new();
  let _ = file.read_to_string(&mut contents);

  let config = toml::from_str(&contents)?;

  Ok(config)
}


pub fn init_config(name: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
  let repo = match Repository::discover(".") {
    Ok(repo) => repo,
    Err(_) => Err("couldn't find git repository")?,
  };

  let origin = match repo.find_remote("origin") {
    Ok(origin) => origin,
    Err(_) => Err("couldn't find remote origin")?,
  };
  let url = origin.url().expect("couldn't get remote url");
  let repo_name = url.split("/").last().unwrap_or("unknown_repo").to_string();

  let commit = repo.head().expect("couldn't get HEAD commit")
    .peel_to_commit().expect("couldn't peel to commit");
  let base = commit.id().to_string();

  let config = ExperimentConfig {
    name: name.unwrap_or(repo_name),
    repo: url.to_string(),
    base,
    treatments: vec![],
    trigger: Trigger::HashMod {
      key: "default".to_string(),
      modulus: 1,
      threshold: 0,
    },
  };

  let toml_string = toml::to_string(&config).expect("couldn't serialize config");
  let mut file = match File::create(CONFIG_FILE) {
    Ok(file) => file,
    Err(e) => Err(format!("couldn't create config file: {}", e))?,
  };

  match file.write_all(toml_string.as_bytes()) {
    Ok(_) => Ok(()),
    Err(e) => Err(Box::new(e))?,
  }
}