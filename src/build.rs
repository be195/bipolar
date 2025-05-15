use git2::{build::CheckoutBuilder, ObjectType, Oid, Repository};
use rand::{seq::SliceRandom, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Serialize, Deserialize};
use std::{collections::HashMap, fs::File, io::Read, path::{Path, PathBuf}, process::Command};
use crate::config;
use crate::utils;

const MANIFEST_FILE: &str = "manifest.toml";

pub fn get_build_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut base = config::get_base()?;
    base.push(".bipolar");

    if !base.exists() {
        std::fs::create_dir_all(&base)?;
    }

    Ok(base)
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub assignment: config::Assignment,
    pub shard_count: usize,
    pub minmax: (usize, usize),
}

pub fn get_manifest_path() -> Result<String, Box<dyn std::error::Error>> {
    let mut path = get_build_dir()?;
    path.push(MANIFEST_FILE);

    Ok(path.to_str().unwrap_or("unknown").to_string())
}

pub fn compare_manifest(
    manifest: &Manifest,
) -> Result<bool, Box<dyn std::error::Error>> {
    let path = get_manifest_path()?;
    let mut file = File::open(path)?;

    let mut contents = String::new();
    let _ = file.read_to_string(&mut contents);
    let current_manifest: Manifest = toml::from_str(&contents)?;

    if current_manifest.shard_count != manifest.shard_count {
        return Ok(false);
    }

    if current_manifest.minmax != manifest.minmax {
        return Ok(false);
    }

    Ok(false) // TODO: this is going to be always nuclear lol
}

pub fn form_manifest(config: &config::ExperimentConfig) -> Manifest {
    return Manifest {
        assignment: config.assignment.clone(),
        shard_count: config.shard_count,
        minmax: config.minmax,
    };
}

pub fn clone_control_repo(config: &config::ExperimentConfig, path: &PathBuf) -> Result<PathBuf, Box<dyn std::error::Error>> {
    println!("cloning control repo from {}", config.repo);

    let control_repo_path = path.join(".control");
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

pub fn build(config: &config::ExperimentConfig, nuclear: Option<bool>) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_build_dir()?;
    let manifest = form_manifest(config);

    if compare_manifest(&manifest).unwrap_or(false) && !nuclear.unwrap_or(false) {
        println!("manifest is up to date, skipping build"); // TODO:
        return Ok(());
    }

    println!("NUKING EVERYTHING");
    std::fs::remove_dir_all(&path)?;

    let control_repo_path = clone_control_repo(config, &path)?;

    let mut storage = HashMap::new();
    for i in config.minmax.0..config.minmax.1 {
        let shard_path = path.join(format!("shard_{}", i));

        println!("copying control to {}", shard_path.display());

        utils::copy_dir_recursive(&control_repo_path, &shard_path)?;

        let shard_repo = Repository::open(&shard_path)?;
        storage.insert(i, shard_repo);
    }

    for treatment in &config.treatments {
        let mut name = "";

        match treatment {
            config::Treatment::Branch(branch_treatment) =>
                name = &branch_treatment.name,
            config::Treatment::Commit(commit_treatment) =>
                name = &commit_treatment.name,
            config::Treatment::Patch(patch_treatment) =>
                name = &patch_treatment.name,
        }

        // get shard ids incrementally
        let mut shard_ids = (config.minmax.0..config.minmax.1).collect();

        match &config.assignment.strategy {
            config::StrategyType::Random(random) =>
                shard_ids = shuffled_shards(&random.seed, name, config.minmax.0, config.minmax.1),
            _ => panic!("not a random assignment"),
        }

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

    Ok(())
}