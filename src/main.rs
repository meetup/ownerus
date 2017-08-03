extern crate clap;
extern crate futures;
extern crate futures_cpupool;
extern crate glob;
extern crate treeline;

use clap::{App, Arg};
use std::process::Command;
use std::io::{BufRead, BufReader, Result};
use std::iter::Iterator;
use std::collections::HashMap;
use futures::{Future, Stream};
use glob::Pattern;
use futures_cpupool::CpuPool;

fn main() {
    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .about("Suggests likely code owners of git versioned paths")
        .arg(
            Arg::with_name("exclude")
                .help("Excludes a glob matched path from results")
                .takes_value(true)
                .multiple(true)
                .short("e")
                .long("exclude"),
        )
        .arg(
            Arg::with_name("PATH")
                .help("Sets the input file to use")
                .index(1),
        )
        .get_matches();
    let excludes = args.values_of("exclude").and_then(|excludes| {
        excludes
            .map(|exclude| Pattern::new(exclude).ok())
            .collect::<Option<Vec<_>>>()
    });
    let filter = args.value_of("PATH").and_then(
        |path| Pattern::new(path).ok(),
    );
    match Command::new("git").arg("ls-files").output() {
        Ok(files) => {
            let pool = CpuPool::new(4);
            let paths = BufReader::new(&files.stdout[..])
                .lines()
                .filter_map(Result::ok)
                .filter(|line| {
                    if let Some(ref exs) = excludes {
                        if exs.iter().any(|ex| ex.matches(&line)) {
                            return false;
                        }
                    }
                    if let Some(ref f) = filter {
                        return f.matches(&line);
                    }
                    true
                });
            for path in paths {
                let (commit_path, contrib_path) = (path.clone(), path.clone());
                let top_commiter = pool.spawn_fn(move || {
                    Command::new("git")
                        .arg("log")
                        .arg("--format='%aE'")
                        .arg(&commit_path)
                        .output()
                        .map(|log| {
                            let lines = BufReader::new(&log.stdout[..]).lines().filter_map(
                                Result::ok,
                            );
                            let counts = lines.fold(HashMap::new(), |mut result, line| {
                                *result.entry(line).or_insert(0) += 1;
                                result
                            });
                            counts.into_iter().max_by_key(|&(_, v)| v)
                        })
                });
                let top_contributor = pool.spawn_fn(move || {
                    Command::new("git")
                        .arg("blame")
                        .arg("-p")
                        .arg(&contrib_path)
                        .output()
                        .map(|blame| {
                            let lines = BufReader::new(&blame.stdout[..])
                                .lines()
                                .filter_map(Result::ok)
                                .filter_map(|mut line| if line.starts_with("committer-mail") {
                                    Some(
                                        line.split_off("committer-mail".len() + 1)
                                            .trim_left_matches("<")
                                            .trim_right_matches(">")
                                            .to_owned(),
                                    )
                                } else {
                                    None
                                });
                            let counts = lines.fold(HashMap::new(), |mut result, line| {
                                *result.entry(line).or_insert(0) += 1;
                                result
                            });
                            counts.into_iter().max_by_key(|&(_, v)| v)
                        })
                });
                let (commiter, contributor) = top_commiter.join(top_contributor).wait().unwrap();
                println!(
                    "{} commits {} blame {}",
                    path,
                    commiter.map(|(k, v)| format!("{} {}", k, v)).unwrap_or(
                        "?".to_owned(),
                    ),
                    contributor.map(|(k, v)| format!("{} {}", k, v)).unwrap_or(
                        "?".to_owned(),
                    )
                );
            }
        }
        Err(err) => {
            println!("err {}", err);
        }
    }
}
