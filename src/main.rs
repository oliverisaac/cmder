use anyhow::{anyhow, Result};
use std::process::{Command, Stdio};
use std::{os, str};

struct FlagAlias {
    flag: String,
    alias: String,
}

struct PostProcessorWhen {
    flag: String,
    is: Option<String>,
    is_not: Option<String>,
    is_set: Option<bool>,
}

struct PostProcessor {
    when_any: Option<Vec<PostProcessorWhen>>,
    set_flags: Option<Vec<String>>,
    run: Vec<String>,
}

struct CommandConfig {
    name: String,
    exe: String,
    flags_aliases: Vec<FlagAlias>,
    post_processors: Vec<PostProcessor>,
}

struct ConfigFile {
    commands: Vec<CommandConfig>,
}

impl ConfigFile {
    fn get_command(&self, name: &str) -> Option<&CommandConfig> {
        self.commands.iter().find(|c| c.name == name)
    }
}
// impement some tests:
#[cfg(test)]
mod test {
    use super::*;

    fn get_example_config() -> ConfigFile {
        let config = ConfigFile {
            commands: vec![CommandConfig {
                name: "kubectl".to_string(),
                exe: "kubectl".to_string(),
                flags_aliases: vec![FlagAlias {
                    flag: "kubectl".to_string(),
                    alias: "x".to_string(),
                }],
                post_processors: vec![PostProcessor {
                    when_any: Some(vec![
                        PostProcessorWhen {
                            flag: "--output".to_string(),
                            is: Some("yq".to_string()),
                            is_not: None,
                            is_set: None,
                        },
                        PostProcessorWhen {
                            flag: "-o".to_string(),
                            is: Some("yq".to_string()),
                            is_not: None,
                            is_set: None,
                        },
                    ]),
                    set_flags: Some(vec!["--output".to_string(), "yaml".to_string()]),
                    run: vec!["yq".to_string(), "-P".to_string(), ".".to_string()],
                }],
            }],
        };
        return config;
    }

    #[test]
    fn test_get_command() {
        let config = get_example_config();
        let command = config.get_command("kubectl").unwrap();
        assert_eq!(command.name, "kubectl");
    }

    #[test]
    fn test_unable_to_get_command() {
        let config = get_example_config();
        assert!(config.get_command("missing").is_none());
    }

    #[test]
    fn test_use_flag_aliases() {
        let flag_aliases = vec![FlagAlias {
            flag: "--context".to_string(),
            alias: "-x".to_string(),
        }];
        let args = vec!["-x".to_string(), "example".to_string()];
        let new_args = configure_flag_aliases(&flag_aliases, &args);
        assert_eq!(
            new_args,
            vec!["--context".to_string(), "example".to_string()]
        );
    }

    #[test]
    fn test_stop_parsing_args() {
        let flag_aliases = vec![FlagAlias {
            flag: "--context".to_string(),
            alias: "-x".to_string(),
        }];
        let args = vec!["-x".to_string(), "--".to_string(), "-x".to_string()];
        let new_args = configure_flag_aliases(&flag_aliases, &args);
        assert_eq!(
            new_args,
            vec!["--context".to_string(), "--".to_string(), "-x".to_string(),]
        );
    }

    #[test]
    fn test_short_equals() {
        let flag_aliases = vec![FlagAlias {
            flag: "--context".to_string(),
            alias: "-x".to_string(),
        }];
        let args = vec!["-x=example".to_string()];
        let new_args = configure_flag_aliases(&flag_aliases, &args);
        assert_eq!(new_args, vec!["--context=example".to_string()]);
    }

    #[test]
    fn test_do_not_make_mistakes() {
        let flag_aliases = vec![FlagAlias {
            flag: "--context".to_string(),
            alias: "-x".to_string(),
        }];
        let args = vec!["-xcalibur=example".to_string()];
        let new_args = configure_flag_aliases(&flag_aliases, &args);
        assert_eq!(new_args, vec!["-xcalibur=example".to_string()]);
    }
}

fn configure_flag_aliases(flag_aliases: &Vec<FlagAlias>, args: &Vec<String>) -> Vec<String> {
    let mut new_args = Vec::new();
    let mut stop_parsing_args = false;

    for arg in args {
        if stop_parsing_args || arg == "--" {
            stop_parsing_args = true;
            new_args.push(arg.clone());
            continue;
        }

        let flag_alias = flag_aliases
            .iter()
            .find(|f| f.alias == *arg || arg.starts_with(format!("{}=", f.alias).as_str()));
        match flag_alias {
            Some(f) => new_args.push(arg.replacen(f.alias.as_str(), f.flag.as_str(), 1)),
            None => new_args.push(arg.clone()),
        };
    }
    return new_args;
}

// Returns the post processor to run and the updated args
fn get_post_processor(
    args: &Vec<String>,
    post_processor_config: &Vec<PostProcessor>,
) -> (Option<Command>, Vec<String>) {
    let mut new_args: Vec<String> = Vec::new();
    let num_args = args.len();
    for i in 0..num_args {
        let mut this_arg = args.get(i).unwrap();

        let mut next_arg = "";
        if let Some(next) = args.get(i + 1) {
            next_arg = next;
        }

        for cfg in post_processor_config {
            if cfg.when_any.is_none() {
                continue;
            }

            let when_any = cfg.when_any.as_ref().unwrap();

            let when_any_match = when_any.iter().find(|w| {
                // If checking flag is set to a specific value
                if let Some(is_value) = &w.is {
                    return w.flag == *this_arg && is_value == next_arg
                        || format!("{}={}", w.flag, is_value) == *this_arg;
                }

                // If checking flag is NOT set to a specific value
                if let Some(is_not_value) = &w.is_not {
                    return w.flag == *this_arg && is_not_value != next_arg
                        || (this_arg.starts_with(format!("{}=", w.flag).as_str())
                            && format!("{}={}", w.flag, is_not_value) != *this_arg);
                }

                // if checking if flag is just set
                if let Some(is_set) = w.is_set {
                    return w.flag == *this_arg && is_set == true;
                }

                return false;
            });

            if let Some(when_any_match) = when_any_match {
                if let Some(set_flags) = &cfg.set_flags {
                    for a in set_flags {
                        new_args.push(a.clone());
                    }
                }

                let captured_value =
                    if this_arg.starts_with(format!("{}=", when_any_match.flag).as_str()) {
                        this_arg
                            .trim_start_matches(format!("{}=", when_any_match.flag).as_str())
                            .to_owned()
                    } else {
                        next_arg.to_owned()
                    };

                let mut remaining_args = i + 1;
                if when_any_match.is_set != Some(true)
                    && !this_arg.starts_with(format!("{}=", when_any_match.flag).as_str())
                {
                    remaining_args = remaining_args + 1;
                }
                if remaining_args < num_args {
                    for j in remaining_args..num_args {
                        new_args.push(args.get(j).unwrap().clone());
                    }
                }

                if cfg.run.len() > 0 {
                    let pp_cmd = cfg.run.get(0).unwrap();
                    let pp_args: Vec<&String> = cfg
                        .run
                        .iter()
                        .skip(1)
                        .map(|v| {
                            if v == "$VALUE" {
                                return &captured_value;
                            }
                            return v;
                        })
                        .collect();
                    let mut post_processor = Command::new(pp_cmd.clone());
                    post_processor.args(pp_args);
                    return (Some(post_processor), new_args);
                }
                return (None, new_args);
            }
        }
        new_args.push(this_arg.clone());
    }
    return (None, args.clone());
}

fn run_command(command: &mut Command) -> Result<()> {
    let command_child = command.spawn()?;
    command_child.wait_with_output()?;
    return Ok(());
}

fn run_command_with_post_processor(
    command: &mut Command,
    post_processor: &mut Command,
) -> Result<()> {
    let command_child = command.stdout(Stdio::piped()).spawn()?;
    let post_processor_child = post_processor
        .stdin(Stdio::from(command_child.stdout.unwrap()))
        .spawn()
        .unwrap();
    post_processor_child.wait_with_output()?;
    return Ok(());
}

fn main() -> Result<()> {
    let config = ConfigFile {
        commands: vec![CommandConfig {
            name: "kubectl".to_string(),
            exe: "kubectl".to_string(),
            flags_aliases: vec![FlagAlias {
                flag: "--context".to_string(),
                alias: "-x".to_string(),
            }],
            post_processors: vec![
                PostProcessor {
                    when_any: Some(vec![
                        PostProcessorWhen {
                            flag: "-o".to_string(),
                            is: Some("yq".to_string()),
                            is_not: None,
                            is_set: None,
                        },
                        PostProcessorWhen {
                            flag: "--output".to_string(),
                            is: Some("yq".to_string()),
                            is_not: None,
                            is_set: None,
                        },
                    ]),
                    set_flags: Some(vec!["--output".to_string(), "yaml".to_string()]),
                    run: vec!["yq".to_string(), "-P".to_string()],
                },
                PostProcessor {
                    when_any: Some(vec![PostProcessorWhen {
                        flag: "--yq".to_string(),
                        is: None,
                        is_not: Some("".to_string()),
                        is_set: None,
                    }]),
                    set_flags: Some(vec!["--output".to_string(), "yaml".to_string()]),
                    run: vec!["yq".to_string(), "-P".to_string(), "$VALUE".to_string()],
                },
                PostProcessor {
                    when_any: Some(vec![PostProcessorWhen {
                        flag: "--jq".to_string(),
                        is: None,
                        is_not: Some("".to_string()),
                        is_set: None,
                    }]),
                    set_flags: Some(vec!["--output".to_string(), "json".to_string()]),
                    run: vec!["jq".to_string(), "-r".to_string(), "$VALUE".to_string()],
                },
            ],
        }],
    };

    let commandName = std::env::args().nth(1);
    if commandName.is_none() {
        return Err(anyhow!("No command name provided"));
    }
    let commandName = commandName.unwrap();

    let commandConfig = config.get_command(commandName.as_str());
    if commandConfig.is_none() {
        return Err(anyhow!("Command {} not found in config", commandName));
    }
    let commandConfig = commandConfig.unwrap();

    let given_args = std::env::args().skip(2);
    let args_to_use = configure_flag_aliases(&commandConfig.flags_aliases, &given_args.collect());

    let (mut post_processor, mut args_to_use) =
        get_post_processor(&args_to_use, &commandConfig.post_processors);

    let mut command = Command::new(commandConfig.exe.clone());
    command.args(args_to_use);

    if let Some(post_processor) = post_processor.as_mut() {
        run_command_with_post_processor(&mut command, post_processor)?;
    } else {
        run_command(&mut command)?;
    }

    return Ok(());
}
