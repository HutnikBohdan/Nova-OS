#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent<'a> {
    Help,
    Status,
    History,
    List,
    Read { path: &'a str },
    Write { path: &'a str, text: &'a str },
    Append { path: &'a str, text: &'a str },
    Delete { path: &'a str },
    BuildRun { path: &'a str },
    Rollback { action_id: u32 },
    Run { program: &'a str, args: &'a str },
    Ask { prompt: &'a str },
    Unknown { input: &'a str },
}

pub fn parse(input: &str) -> Intent<'_> {
    let input = input.trim();
    if input.is_empty() || input == "help" || input == "допомога" {
        return Intent::Help;
    }
    if input == "status" || input == "стан" {
        return Intent::Status;
    }
    if input == "history" || input == "історія" {
        return Intent::History;
    }
    if input == "ls" || input == "dir" || input == "файли" {
        return Intent::List;
    }
    let (verb, rest) = split_once_space(input);
    match verb {
        "cat" | "read" | "прочитай" => Intent::Read { path: rest.trim() },
        "rm" | "delete" | "видали" => Intent::Delete { path: rest.trim() },
        "build" | "збери" | "скомпілюй" => Intent::BuildRun { path: rest.trim() },
        "rollback" | "відкотити" => rest
            .trim()
            .parse::<u32>()
            .map(|action_id| Intent::Rollback { action_id })
            .unwrap_or(Intent::Unknown { input }),
        "run" | "запусти" => {
            let (program, args) = split_once_space(rest.trim());
            Intent::Run {
                program,
                args: args.trim(),
            }
        }
        "write" | "створи" | "запиши" => {
            let (path, text) = split_once_space(rest.trim());
            Intent::Write {
                path,
                text: text.trim(),
            }
        }
        "append" | "додай" => {
            let (path, text) = split_once_space(rest.trim());
            Intent::Append {
                path,
                text: text.trim(),
            }
        }
        "ai" | "agent" | "оператор" => Intent::Ask {
            prompt: rest.trim(),
        },
        _ => Intent::Unknown { input },
    }
}

fn split_once_space(value: &str) -> (&str, &str) {
    match value.find(char::is_whitespace) {
        Some(at) => (&value[..at], &value[at..]),
        None => (value, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ukrainian_file_mutation() {
        assert_eq!(
            parse("запиши /нотатка.txt привіт світе"),
            Intent::Write {
                path: "/нотатка.txt",
                text: "привіт світе"
            }
        );
        assert_eq!(
            parse("додай /нотатка.txt !"),
            Intent::Append {
                path: "/нотатка.txt",
                text: "!"
            }
        );
    }

    #[test]
    fn parses_ukrainian_program_launch() {
        assert_eq!(
            parse("запусти echo привіт"),
            Intent::Run {
                program: "echo",
                args: "привіт"
            }
        );
    }

    #[test]
    fn parses_ukrainian_build_and_run() {
        assert_eq!(
            parse("збери /проекти/привіт.nv"),
            Intent::BuildRun {
                path: "/проекти/привіт.nv"
            }
        );
    }

    #[test]
    fn parses_guardian_history_and_rollback() {
        assert_eq!(parse("історія"), Intent::History);
        assert_eq!(parse("відкотити 17"), Intent::Rollback { action_id: 17 });
        assert!(matches!(parse("відкотити abc"), Intent::Unknown { .. }));
    }
}
