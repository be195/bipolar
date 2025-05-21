use crate::{config, build, utils};
use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, thread, time::Duration};

pub fn run(config: &config::ExperimentConfig) -> Result<(), Box<dyn std::error::Error>> {
    if config.hooks.run.is_none() {
        return Err("no run hook found".into());
    }

    if let Some(environment) = &config.environment {
        for (key, value) in environment {
            std::env::set_var(key, value);
        }
    }

    let mut children = Vec::new();

    for shard in config.minmax.0..config.minmax.1 {
        let shard_dir = build::get_shard_dir(shard)?;

        let hook = config.hooks.run.clone();
        if let Some(hook) = hook {
            println!("running for shard {}", shard);
            let child = utils::run_command_string(&hook, &shard_dir.to_str().unwrap(), true)?;
            children.push(child);

            thread::sleep(Duration::from_millis(500));
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    println!("waiting for ctrl-c...");

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).unwrap();

    while running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }

    println!("killing children");

    for mut child in children {
        child.kill().unwrap();
    }

    Ok(())
}