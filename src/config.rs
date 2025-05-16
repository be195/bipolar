use serde::{Serialize, Deserialize};
use std::{collections::HashMap, fs::File, io::{Read, Write}, path::PathBuf};
use git2::Repository;

const CONFIG_FILE: &str = "bipolar.toml";

#[derive(Debug, Deserialize, Serialize)]
pub struct ExperimentConfig {
    pub name: String,
    pub repo: String,
    pub base: String,
    pub treatments: Vec<Treatment>,
    pub assignment: Assignment,
    pub hooks: Hooks,
    pub templating: Option<Templating>,

    // if we have multiple servers, we can configure each instance of
    // bipolar to have a minimum and maximum number of shards, but the
    // shard count is always the same for all instances
    pub shard_count: usize,
    pub minmax: (usize, usize),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Templating {
    pub path: String,
    pub config: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Hooks {
    pub control_build: Option<String>,
    pub build: Option<String>,
    pub run: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct DefaultStrategy {}

// assigned on build time
#[derive(Debug, Deserialize, Serialize)]
pub struct RandomStrategy {
    pub seed: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum StrategyType {
    Proxy(DefaultStrategy),
    Random(RandomStrategy),
}

impl StrategyType {
    pub fn clone(&self) -> StrategyType {
        match self {
            StrategyType::Proxy(_) => StrategyType::Proxy(DefaultStrategy {}),
            StrategyType::Random(random) => StrategyType::Random(RandomStrategy {
                seed: random.seed,
            }),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Assignment {
    pub split: HashMap<String, u8>,
    pub strategy: StrategyType,
}

impl Assignment {
    pub fn clone(&self) -> Assignment {
        Assignment {
            split: self.split.clone(),
            strategy: self.strategy.clone(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BranchTreatment {
    pub name: String,
    pub ref_: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CommitTreatment {
    pub name: String,
    pub ref_: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PatchTreatment {
    pub name: String,
    pub patch: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Treatment {
    Branch(BranchTreatment),
    Commit(CommitTreatment),
    Patch(PatchTreatment),
}

pub fn get_base() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let repo = Repository::discover(".")?;

    let mut path = repo.path().to_path_buf();
    path.pop();
    Ok(path)
}

pub fn get_config_path() -> Result<String, Box<dyn std::error::Error>> {
    let mut path = get_base()?;
    path.push(CONFIG_FILE);

    Ok(path.to_str().unwrap_or("unknown").to_string())
}

pub fn load_config() -> Result<ExperimentConfig, Box<dyn std::error::Error>> {
    let path = get_config_path()?;
    let mut file = File::open(path)?;

    let mut contents = String::new();
    let _ = file.read_to_string(&mut contents);

    let config = toml::from_str(&contents)?;

    Ok(config)
}

pub fn init_config(name: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let repo = Repository::discover(".")?;

    let origin = repo.find_remote("origin")?;
    let url = origin.url().expect("couldn't get remote url");
    let repo_name = url.split("/").last().unwrap_or("unknown_repo").to_string();

    let commit = repo.head().expect("couldn't get HEAD commit")
        .peel_to_commit().expect("couldn't peel to commit");
    let base = commit.id().to_string();

    let config = ExperimentConfig {
        name: name.unwrap_or(repo_name),
        repo: url.to_string(),
        base,
        hooks: Hooks {
            control_build: None,
            build: None,
            run: None,
        },
        templating: None,
        treatments: vec![],
        assignment: Assignment {
            split: HashMap::new(),
            strategy: StrategyType::Random(RandomStrategy { seed: 0 }),
        },
        shard_count: 1,
        minmax: (0, 0),
    };

    match save_config(&config) {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("couldn't save config: {}", e))?,
    }
}

pub fn try_load_config() -> ExperimentConfig {
    match load_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("error loading config: {}", e);
            std::process::exit(1);
        }
    }
}

pub fn save_config(config: &ExperimentConfig) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_config_path()?;

    let toml_string = toml::to_string(&config).expect("couldn't serialize config");
    let mut file = File::create(path)?;

    match file.write_all(toml_string.as_bytes()) {
        Ok(_) => Ok(()),
        Err(e) => Err(Box::new(e))?,
    }
}