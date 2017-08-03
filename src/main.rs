extern crate clap;
extern crate futures;
extern crate futures_cpupool;
extern crate glob;
extern crate treeline;

use std::process::Command;
use std::fmt;
use std::io::{self, BufRead, BufReader};
use std::iter::Iterator;
use std::collections::HashMap;

use clap::{App, Arg};
use futures::Future;
use futures_cpupool::CpuPool;
use glob::Pattern;

struct GitPath {
    path: String,
    top_commiter: Option<(String, usize)>,
    top_contributor: Option<(String, usize)>,
}

impl fmt::Display for GitPath {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.write_fmt(format_args!(
            "{} commits {} blame {}",
            self.path,
            self.top_commiter
                .clone()
                .map(|(k, v)| format!("{} {}", k, v))
                .unwrap_or("?".to_owned()),
            self.top_contributor
                .clone()
                .map(|(k, v)| format!("{} {}", k, v))
                .unwrap_or("?".to_owned())
        ))
    }
}

type GitStatsFuture = Box<Future<Item = Option<(String, usize)>, Error = ::std::io::Error>>;

/// top committers by commit count history
fn top_commiter(pool: &CpuPool, path: &str) -> GitStatsFuture {
    let path = path.to_owned();
    pool.spawn_fn(move || {
        Command::new("git")
            .arg("log")
            .arg("--format=%aE")
            .arg(&path)
            .output()
            .map(|log| {
                let lines = BufReader::new(&log.stdout[..]).lines().filter_map(
                    io::Result::ok,
                );
                let counts = lines.fold(HashMap::new(), |mut result, line| {
                    *result.entry(line).or_insert(0) += 1;
                    result
                });
                counts.into_iter().max_by_key(|&(_, v)| v)
            })
    }).boxed()
}

/// top contributor by blame lines
fn top_contributor(pool: &CpuPool, path: &str) -> GitStatsFuture {
    let path = path.to_owned();
    pool.spawn_fn(move || {
        Command::new("git")
            .arg("blame")
            .arg("-p")
            .arg(&path)
            .output()
            .map(|blame| {
                let lines = BufReader::new(&blame.stdout[..])
                    .lines()
                    .filter_map(io::Result::ok)
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
    }).boxed()
}

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
                .filter_map(io::Result::ok)
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
                let (commiter, contributor) = top_commiter(&pool, &path)
                    .join(top_contributor(&pool, path.as_str()))
                    .wait()
                    .unwrap();
                let git_path = GitPath {
                    path: path,
                    top_commiter: commiter,
                    top_contributor: contributor,
                };
                println!("{}", git_path);
            }
        }
        Err(err) => {
            println!("err {}", err);
        }
    }
}
