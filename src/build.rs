use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Serialize, Deserialize};
use std::{collections::HashMap, fs::File, io::Read, path::PathBuf};
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
    let control_repo = git2::Repository::clone(
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

fn lucky(random: &config::RandomStrategy, treatment: &str) -> f64 {
    let seed = treatment.chars().fold(0, |acc, c| acc + c as u64);
    let mut rng = ChaCha8Rng::seed_from_u64(seed + random.seed);
    let random_number = rng.random_range(0.0..1.0);

    return random_number;
}

pub fn apply_treatment(
    shard_repo: &git2::Repository,
    treatment: &config::Treatment,
) -> Result<(), Box<dyn std::error::Error>> {
    match treatment {
        config::Treatment::Branch(branch_treatment) => {
            let branch = &branch_treatment.ref_;
            let reference = shard_repo.find_reference(&format!("refs/remotes/origin/{branch}"))?;
            let object = reference.peel(git2::ObjectType::Commit)?;
            shard_repo.reset(&object, git2::ResetType::Hard, None)?;
            shard_repo.set_head(&format!("refs/heads/{branch}"))?;
        }
        config::Treatment::Commit(commit_treatment) => {
            let oid = git2::Oid::from_str(&commit_treatment.ref_)?;
            let object = shard_repo.find_object(oid, Some(git2::ObjectType::Commit))?;

            shard_repo.reset(
              &object,
              git2::ResetType::Hard,
              Some(git2::build::CheckoutBuilder::new().force())
            )?;
            shard_repo.set_head_detached(oid)?;
        },
        config::Treatment::Patch(_) => {
            panic!("not implemented yet");
        },
    }

    Ok(())
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

        let shard_repo = git2::Repository::open(&shard_path)?;
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

      for i in config.minmax.0..config.minmax.1 {
        let shard_repo = storage.get(&i).unwrap();

        println!("_______ SHARD {} ________", i);

        let mut split = 0;
        if let Some(s) = config.assignment.split.get(name) {
            split = *s;
        } else {
            println!("⚠️ no split for treatment {}, skipping", name);
            continue;
        }

        let mut cut = i as f64 / config.shard_count as f64;
        match &config.assignment.strategy {
            config::StrategyType::Random(random) => {
                cut = lucky(random, &format!("{name}:{i}"));
                println!("random assignment for treatment {}: {}", name, cut);
            }
            _ => panic!("not a random assignment"),
        }

        if cut < split as f64 / 100.0 {
            println!("💉 applying treatment {} to shard {}", name, i);
            apply_treatment(&shard_repo, treatment)?;
        } else {
            println!("🚷 skipping treatment {} for shard {}", name, i);
        }
      }
    }

    Ok(())
}