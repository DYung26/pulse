//! `pulse` — CLI client for pulse-daemon. Talks over the same socket
//! any UI would use (see docs/protocol.md); this is the reference
//! client and the fastest way to add/inspect notes without a GUI.

use std::collections::HashMap;
use std::process::ExitCode;

use pulse_core::client;
use pulse_core::protocol::Request;
use uuid::Uuid;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();

    let result = match args.get(1).map(String::as_str) {
        Some("add") => cmd_add(&args[2..]),
        Some("list") => cmd_list(&args[2..]),
        Some("update") => cmd_update(&args[2..]),
        Some("delete") => cmd_delete(&args[2..]),
        Some("show") => cmd_show(),
        Some("get-interval") => cmd_get_interval(),
        Some("set-interval") => cmd_set_interval(&args[2..]),
        _ => {
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pulse: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage:\n  pulse add \"<text>\" [--prop key=value ...]\n  pulse list [--filter key=value]\n  pulse update <id> [--text \"...\"] [--prop key=value ...]\n  pulse delete <id>\n  pulse show\n  pulse get-interval\n  pulse set-interval <seconds>"
    );
}

/// Parse repeated `--prop key=value` pairs into a map. `None` means
/// no `--prop` flags were present at all (distinct from present but
/// empty, which can't actually happen given the flag's own shape, but
/// keeping the distinction makes intent explicit at call sites like
/// `update`, where "no --prop given" must mean "don't touch
/// properties" rather than "clear properties").
fn parse_props(args: &[String]) -> Result<Option<HashMap<String, String>>, String> {
    let mut props = HashMap::new();
    let mut saw_any = false;
    let mut i = 0;

    while i < args.len() {
        if args[i] == "--prop" {
            saw_any = true;
            let pair = args
                .get(i + 1)
                .ok_or_else(|| "--prop requires a key=value argument".to_string())?;
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("--prop value '{pair}' must be in key=value form"))?;
            props.insert(key.to_string(), value.to_string());
            i += 2;
        } else {
            i += 1;
        }
    }

    Ok(if saw_any { Some(props) } else { None })
}

fn cmd_add(args: &[String]) -> Result<(), String> {
    let text = args
        .first()
        .ok_or("pulse add requires text, e.g. pulse add \"my note\"")?
        .clone();

    let properties = parse_props(&args[1..])?.unwrap_or_default();

    let request = Request::AddNote { text, properties };
    let response = client::send(&request).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn cmd_list(args: &[String]) -> Result<(), String> {
    let filter = if let Some(pos) = args.iter().position(|a| a == "--filter") {
        let pair = args
            .get(pos + 1)
            .ok_or("--filter requires a key=value argument")?;
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--filter value '{pair}' must be in key=value form"))?;
        let mut map = HashMap::new();
        map.insert(key.to_string(), value.to_string());
        Some(map)
    } else {
        None
    };

    let request = Request::ListNotes { filter };
    let response = client::send(&request).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn cmd_update(args: &[String]) -> Result<(), String> {
    let id_str = args.first().ok_or("pulse update requires an id")?;
    let id: Uuid = id_str
        .parse()
        .map_err(|_| format!("'{id_str}' is not a valid note id"))?;

    let mut rest = &args[1..];
    let text = if rest.first().map(String::as_str) == Some("--text") {
        let value = rest.get(1).ok_or("--text requires a value")?.clone();
        rest = &rest[2..];
        Some(value)
    } else {
        None
    };

    let properties = parse_props(rest)?;

    if text.is_none() && properties.is_none() {
        return Err("update requires at least --text or one --prop".to_string());
    }

    let request = Request::UpdateNote {
        id,
        text,
        properties,
    };
    let response = client::send(&request).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn cmd_delete(args: &[String]) -> Result<(), String> {
    let id_str = args.first().ok_or("pulse delete requires an id")?;
    let id: Uuid = id_str
        .parse()
        .map_err(|_| format!("'{id_str}' is not a valid note id"))?;

    let request = Request::DeleteNote { id };
    let response = client::send(&request).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn cmd_show() -> Result<(), String> {
    let response = client::send(&Request::ShowNow).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn cmd_get_interval() -> Result<(), String> {
    let response = client::send(&Request::GetInterval).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn cmd_set_interval(args: &[String]) -> Result<(), String> {
    let seconds_str = args
        .first()
        .ok_or("pulse set-interval requires a number of seconds, e.g. pulse set-interval 150")?;
    let seconds: u64 = seconds_str
        .parse()
        .map_err(|_| format!("'{seconds_str}' is not a valid number of seconds"))?;

    let request = Request::SetInterval { seconds };
    let response = client::send(&request).map_err(|e| e.to_string())?;
    print_response(&response);
    Ok(())
}

fn print_response(response: &serde_json::Value) {
    match serde_json::to_string_pretty(response) {
        Ok(pretty) => println!("{pretty}"),
        Err(_) => println!("{response}"),
    }
}
