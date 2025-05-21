use git2::{build::CheckoutBuilder, ObjectType, Oid, Repository};
use rand::{seq::SliceRandom, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Serialize, Deserialize};
use tera::{Tera, Context};
use std::{collections::HashMap, fs, io::{Read, Write}, path::{Path, PathBuf}, process::Command};
use crate::{config, utils};
use walkdir::WalkDir;

pub const CONTROL_REPO_DIR : &str = ".control";
pub const LOCKFILE_FILE: &str = "lockfile.toml";
pub const BUILD_DIR: &str = ".bipolar";

#[derive(Debug, Deserialize, Serialize)]
pub struct LockFile {
    pub assignment: config::Assignment,
    pub base: String,
    pub repo: String,
    pub shard_count: usize,
    pub minmax: (usize, usize),
    pub applied: HashMap<String, Vec<usize>>,
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

pub fn get_build_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut base = config::get_base()?;
    base.push(BUILD_DIR);

    if !base.exists() {
        std::fs::create_dir_all(&base)?;
    }

    Ok(base)
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
    let mut file = fs::File::open(path)?;

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
        applied: HashMap::new(),
    };
}

fn write_lockfile(lockfile: &LockFile) -> Result<(), Box<dyn std::error::Error>> {
    let path = get_lockfile_path()?;
    let mut file = fs::File::create(path)?;

    let contents = toml::to_string(&lockfile).expect("couldn't save lockfile, wtf");

    match file.write_all(contents.as_bytes()) {
        Ok(_) => Ok(()),
        Err(e) => Err(Box::new(e))?,
    }
}

pub fn get_shard_dir(shard: usize) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut path = get_build_dir()?;
    path.push(format!("shard_{}", shard));

    Ok(path)
}

fn populate_shard_repos(
    config: &config::ExperimentConfig,
    control_repo_path: &Path,
) -> Result<HashMap<usize, Repository>, Box<dyn std::error::Error>> {
    let mut storage = HashMap::new();

    for i in config.minmax.0..config.minmax.1 {
        let shard_path = get_shard_dir(i)?;
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

    if config.hooks.control_build.is_some() {
        println!("🔨 building control repo");
        utils::run_command_string(
            &config.hooks.control_build.as_ref().unwrap(),
            control_repo_path.to_str().expect("couldn't get control repo path, wtf"),
            false,
        )?;
    }

    Ok(control_repo_path)
}

fn merge_commit_into(repo: &Repository, commit: &git2::Commit) -> Result<(), Box<dyn std::error::Error>> {
    let head_commit = repo.head()?.peel_to_commit()?;
    let ancestor = repo.merge_base(head_commit.id(), commit.id())?;
    let ancestor_commit = repo.find_commit(ancestor)?;

    let head_tree = head_commit.tree()?;
    let commit_tree = commit.tree()?;
    let ancestor_tree = ancestor_commit.tree()?;

    let mut index = repo.merge_trees(&ancestor_tree, &head_tree, &commit_tree, None)?;

    if index.has_conflicts() {
        println!("😵‍💫 merge conflict detected");

        let conflicts: Vec<_> = index.conflicts()?.collect::<Result<_, _>>()?;

        for conflict in conflicts {
            if let Some(ours) = conflict.our {
                println!("‼️ using our version of {:?}", ours.path);
                index.add(&ours)?;
            } else if let Some(theirs) = conflict.their {
                println!("‼️ using their version of {:?}", theirs.path);
                index.add(&theirs)?;
            }
        }

        index.write()?;
    }

    let tree_oid = index.write_tree_to(repo)?;
    let tree = repo.find_tree(tree_oid)?;

    let sig = repo.signature()?;
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "bipolar: auto-merge treatment",
        &tree,
        &[&head_commit, &commit],
    )?;

    let mut checkout = CheckoutBuilder::new();
    checkout.force();

    repo.checkout_tree(tree.as_object(), Some(&mut checkout))?;
    repo.set_head("HEAD")?;

    Ok(())
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

            merge_commit_into(shard_repo, &commit)?;
        }

        config::Treatment::Commit(commit_treatment) => {
            let oid = Oid::from_str(&commit_treatment.ref_)?;
            let commit = shard_repo.find_commit(oid)?;

            merge_commit_into(shard_repo, &commit)?;
        }

        config::Treatment::Patch(patch_treatment) => {
            let patch_path = &patch_treatment.patch;
            let status = Command::new("git")
                .arg("apply")
                .arg("--whitespace=fix")
                .arg("--directory")
                .arg(target_dir)
                .arg(patch_path)
                .status()?;

            if !status.success() {
                return Err(format!("failed to apply patch: {:?}", patch_path).into());
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

fn get_home_dir(repo: &Repository) -> PathBuf{
    let mut path = repo.path().to_path_buf();
    path.pop();
    path
}

#[derive(Serialize, Deserialize)]
struct Template {
    shard: usize,
    shard_count: usize,
    custom: HashMap<String, String>,
}

fn template_fill(shard: usize, config: &config::ExperimentConfig, shard_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let template_config = config.templating.as_ref().expect("template config expected, fn not meant to be called");

    let mut context = Context::new();
    // TODO: port?
    context.insert("shard", &shard);
    context.insert("shard_count", &config.shard_count);
    context.insert("custom", &template_config.config);

    let base = config::get_base()?.join(template_config.path.clone());
    let mut tera = Tera::new(&format!("{}/**/*", base.to_str().expect("wtf")))?;

    for entry in WalkDir::new(&base) {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_dir() || path.file_name().unwrap().to_str().unwrap().starts_with(".") {
            continue;
        }

        let template_content = fs::read_to_string(path)?;

        let rendered = tera.render_str(&template_content, &context)?;

        let relative_path = path.strip_prefix(&base)?;
        let output_path = shard_dir.join(relative_path);

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(&output_path, rendered)?;
    }

    Ok(())
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

    let storage = populate_shard_repos(config, &control_repo_path)?;

    for treatment in &config.treatments {
        let name = match treatment {
            config::Treatment::Branch(t) => &t.name,
            config::Treatment::Commit(t) => &t.name,
            config::Treatment::Patch(t) => &t.name,
        };

        let shard_ids = match &config.assignment.strategy {
            config::StrategyType::Random(random) =>
                shuffled_shards(&random.seed, name, 0, config.shard_count),

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
        let iter = shard_ids.iter()
            .take(count)
            .skip(
                lockfile.applied.entry(name.clone()).or_insert(vec![]).len()
            );
        for &i in iter {
            if i < config.minmax.0 || i >= config.minmax.1 {
                continue;
            }

            let shard_repo = storage.get(&i).unwrap();
            let path = get_home_dir(shard_repo);

            println!("💉 applying treatment {} to shard {}", name, i);
            apply_treatment(&shard_repo, treatment, &path)?;
            lockfile.applied.entry(name.clone()).or_insert(vec![]).push(i);
        }
    }

    if config.hooks.build.is_some() || config.templating.is_some() || config.symlinks.is_some() {
        for i in config.minmax.0..config.minmax.1 {
            let path = get_home_dir(storage.get(&i).unwrap());

            if config.hooks.build.is_some() {
                println!("🔨 building shard {}", i);
                utils::run_command_string(
                    &config.hooks.build.as_ref().unwrap(),
                    path.to_str().unwrap_or("unknown"),
                    false,
                )?;
            }

            if config.templating.is_some() {
                println!("📄 filling in config templates for shard {}", i);
                template_fill(i, config, &path)?;
            }

            if let Some(symlinks) = &config.symlinks {
                for symlink in symlinks {
                    let path = get_home_dir(storage.get(&i).unwrap());
                    let symlink_path = path.join(symlink);

                    let default_base = "symlinks/".to_string();
                    let base = config.symlinks_base.as_ref().unwrap_or(&default_base);
                    let original_path = config::get_base()?.join(base).join(symlink);

                    utils::create_symlink_force(&original_path.to_str().unwrap(), &symlink_path.to_str().unwrap())?;
                }
            }
        }
    } else {
        println!("⚠️ templating config and build hook missing!")
    }

    write_lockfile(&lockfile)?;

    println!("🔒 lockfile written to {}", get_lockfile_path()?);
    println!("YOU'RE ALL CAUGHT UP :)");

    Ok(())
}