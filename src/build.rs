use git2::{build::CheckoutBuilder, ObjectType, Oid, Repository};
use rand::{seq::SliceRandom, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Serialize, Deserialize};
use std::{collections::HashMap, fs::File, io::{Read, Write}, path::{Path, PathBuf}, process::Command};
use crate::config;
use crate::utils;

const CONTROL_REPO_DIR : &str = ".control";
const LOCKFILE_FILE: &str = "lockfile.toml";

pub fn get_build_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut base = config::get_base()?;
    base.push(".bipolar");

    if !base.exists() {
        std::fs::create_dir_all(&base)?;
    }

    Ok(base)
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LockFile {
    pub assignment: config::Assignment,
    pub base: String,
    pub repo: String,
    pub shard_count: usize,
    pub minmax: (usize, usize),
}

impl LockFile {
    fn eq(&self, lockfile: &LockFile) -> bool {
        self.base == lockfile.base
            && self.repo == lockfile.repo
            && self.shard_count == lockfile.shard_count
            && self.minmax == lockfile.minmax
            && self.assignment.split.iter().all(|(k, v)| lockfile.assignment.split.get(k).map_or(false, |bv| bv >= v))
            && match (&self.assignment.strategy, &lockfile.assignment.strategy) {
                (config::StrategyType::Random(r1), config::StrategyType::Random(r2)) => r1.seed == r2.seed,
                _ => false,
            }
    }
}

pub fn get_lockfile_path() -> Result<String, Box<dyn std::error::Error>> {
    let mut path = get_build_dir()?;
    path.push(LOCKFILE_FILE);

    Ok(path.to_str().unwrap_or("unknown").to_string())
}

fn compare_lockfile(
    lockfile: &LockFile,
) -> Result<LockFile, Box<dyn std::error::Error>> {
    let path = get_lockfile_path()?;
    let mut file = File::open(path)?;

    let mut contents = String::new();
    let _ = file.read_to_string(&mut contents);
    let current_lockfile: LockFile = toml::from_str(&contents)?;

    if current_lockfile.eq(lockfile) {
        return Ok(current_lockfile);
    }

    Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "not equal")))
}

fn form_lockfile(config: &config::ExperimentConfig) -> LockFile {
    return LockFile {
        assignment: config.assignment.clone(),
        base: config.base.clone(),
        repo: config.repo.clone(),
        shard_count: config.shard_count,
        minmax: config.minmax,
    };
}

fn write_lockfile(lockfile: &LockFile) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_lockfile_path()?;
    let mut file = File::create(path)?;

    let contents = toml::to_string(&lockfile).expect("couldn't save lockfile, wtf");

    match file.write_all(contents.as_bytes()) {
        Ok(_) => Ok(()),
        Err(e) => Err(Box::new(e))?,
    }
}

fn populate_shard_repos(
    config: &config::ExperimentConfig,
    path: &Path,
    control_repo_path: &Path,
) -> Result<HashMap<usize, Repository>, Box<dyn std::error::Error>> {
    let mut storage = HashMap::new();

    for i in config.minmax.0..config.minmax.1 {
        let shard_path = path.join(format!("shard_{}", i));
        if !shard_path.exists() {
            utils::copy_dir_recursive(&control_repo_path, &shard_path)?;
        }
        let repo = Repository::open(&shard_path)?;
        storage.insert(i, repo);
    }

    Ok(storage)
}

pub fn clone_control_repo(config: &config::ExperimentConfig, path: &PathBuf) -> Result<PathBuf, Box<dyn std::error::Error>> {
    println!("cloning control repo from {}", config.repo);

    let control_repo_path = path.join(CONTROL_REPO_DIR);
    let control_repo = Repository::clone(
        &config.repo,
        &control_repo_path,
    )?;

    let (object, reference) = control_repo.revparse_ext(&config.base)?;

    control_repo.checkout_tree(&object, None)?;

    match reference {
        Some(r) => control_repo.set_head(r.name().unwrap()),
        None => control_repo.set_head_detached(object.id())
    }?;

    Ok(control_repo_path)
}

pub fn apply_treatment(
    shard_repo: &Repository,
    treatment: &config::Treatment,
    target_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match treatment {
        config::Treatment::Branch(branch_treatment) => {
            let branch = &branch_treatment.ref_;
            let reference = shard_repo.find_reference(&format!("refs/remotes/origin/{branch}"))?;
            let object = reference.peel(ObjectType::Commit)?;
            let commit = object.into_commit().map_err(|_| "Not a commit")?;
            let tree = commit.tree()?;

            let mut checkout = CheckoutBuilder::new();
            checkout.force().target_dir(target_dir);
            shard_repo.checkout_tree(tree.as_object(), Some(&mut checkout))?;
        }
        config::Treatment::Commit(commit_treatment) => {
            let oid = Oid::from_str(&commit_treatment.ref_)?;
            let commit = shard_repo.find_commit(oid)?;
            let tree = commit.tree()?;

            let mut checkout = CheckoutBuilder::new();
            checkout.force().target_dir(target_dir);
            shard_repo.checkout_tree(tree.as_object(), Some(&mut checkout))?;
        }
        config::Treatment::Patch(patch_treatment) => {
            let patch_path = &patch_treatment.patch;
            let status = Command::new("git")
                .arg("apply")
                .arg("--directory")
                .arg(target_dir)
                .arg(patch_path)
                .status()?;

            if !status.success() {
                return Err("failed to apply patch".into());
            }
        }
    }

    Ok(())
}

fn shuffled_shards(
    seed: &u64,
    treatment_name: &str,
    min: usize,
    max: usize,
) -> Vec<usize> {
    let mut shard_ids: Vec<usize> = (min..max).collect();

    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    treatment_name.hash(&mut hasher);
    let hash = hasher.finish();

    let mut seed_bytes = [0u8; 32];
    seed_bytes[..8].copy_from_slice(&hash.to_le_bytes());

    let mut rng = ChaCha8Rng::from_seed(seed_bytes);
    shard_ids.shuffle(&mut rng);

    shard_ids
}

pub fn build(config: &config::ExperimentConfig, nuclear: bool) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_build_dir()?;
    let mut lockfile = form_lockfile(config);

    let control_repo_path = path.join(CONTROL_REPO_DIR);

    let mut nuke = nuclear;
    match compare_lockfile(&lockfile) {
        Ok(current_lockfile) => lockfile = current_lockfile,
        Err(_) => nuke = true,
    }

    if nuke {
        println!("☢️ nuclear build triggered");

        std::fs::remove_dir_all(&path)?;
        clone_control_repo(config, &path)?;
    }

    let storage = populate_shard_repos(config, &path, &control_repo_path)?;

    for treatment in &config.treatments {
        let name = match treatment {
            config::Treatment::Branch(t) => &t.name,
            config::Treatment::Commit(t) => &t.name,
            config::Treatment::Patch(t) => &t.name,
        };

        let shard_ids = match &config.assignment.strategy {
            config::StrategyType::Random(random) =>
                shuffled_shards(&random.seed, name, config.minmax.0, config.minmax.1),

            _ => (config.minmax.0..config.minmax.1).collect(),
        };

        let mut split = 0;
        if let Some(s) = config.assignment.split.get(name) {
            split = *s;
        } else {
            println!("⚠️ no split for treatment {}, skipping", name);
            continue;
        }

        let count = ((shard_ids.len() as f64) * (split as f64 / 100.0)).round() as usize;

        for &i in shard_ids.iter().take(count) {
            let shard_repo = storage.get(&i).unwrap();
            let mut path = shard_repo.path().to_path_buf();
            path.pop();

            println!("💉 applying treatment {} to shard {}", name, i);
            apply_treatment(&shard_repo, treatment, &path)?;
        }
    }

    write_lockfile(&lockfile)?;

    Ok(())
}