/**
@module PROJECTOR.EDGE.CONNECTION_PROMPTS
Owns interactive prompt helpers and default-filling for human-driven connection and deploy flows.
*/
// @fileimplements PROJECTOR.EDGE.CONNECTION_PROMPTS
use std::error::Error;
use std::io::{self, Write};

use super::args::{ConnectArgs, DeployArgs};
use super::deploy::infer_server_addr;

pub(super) fn prompt_required(prompt: &str) -> Result<String, Box<dyn Error>> {
    loop {
        print!("{prompt}");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let trimmed = input.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_owned());
        }
        eprintln!("value is required");
    }
}

pub(super) fn prompt_with_default(prompt: &str, default: &str) -> Result<String, Box<dyn Error>> {
    print!("{prompt} [{default}]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

pub(super) fn prompt_optional(prompt: &str) -> Result<Option<String>, Box<dyn Error>> {
    print!("{prompt}: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

pub(super) fn prompt_confirm(prompt: &str, default_yes: bool) -> Result<bool, Box<dyn Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return Ok(default_yes);
    }
    Ok(matches!(trimmed.as_str(), "y" | "yes"))
}

pub(super) fn fill_connect_defaults(
    mut args: ConnectArgs,
    interactive: bool,
) -> Result<ConnectArgs, Box<dyn Error>> {
    if interactive {
        if args.profile_id.is_none() {
            args.profile_id = Some(prompt_required("Profile id: ")?);
        }
        if args.server_addr.is_none() {
            args.server_addr = Some(prompt_required("Server address (host:port): ")?);
        }
        if args.ssh_target.is_none() {
            args.ssh_target = prompt_optional("SSH target (optional user@host)")?;
        }
    }

    Ok(ConnectArgs {
        profile_id: Some(
            args.profile_id
                .clone()
                .ok_or("connect requires --id or an interactive terminal")?,
        ),
        server_addr: Some(
            args.server_addr
                .clone()
                .ok_or("connect requires --server or an interactive terminal")?,
        ),
        ssh_target: args.ssh_target,
    })
}

pub(super) fn fill_deploy_defaults(
    mut args: DeployArgs,
    interactive: bool,
) -> Result<DeployArgs, Box<dyn Error>> {
    if interactive {
        if args.profile_id.is_none() {
            args.profile_id = Some(prompt_required("Profile id: ")?);
        }
        if args.ssh_target.is_none() {
            args.ssh_target = Some(prompt_required("SSH target (user@host): ")?);
        }
    }

    let ssh_target = args
        .ssh_target
        .clone()
        .ok_or("deploy requires --ssh or an interactive terminal")?;
    let default_listen_addr = "0.0.0.0:8942".to_owned();
    let listen_addr = if let Some(listen_addr) = args.listen_addr.clone() {
        listen_addr
    } else if interactive {
        prompt_with_default("Remote listen address", &default_listen_addr)?
    } else {
        default_listen_addr
    };
    let default_server_addr = infer_server_addr(&ssh_target, &listen_addr)?;
    let server_addr = if let Some(server_addr) = args.server_addr.clone() {
        server_addr
    } else if interactive {
        prompt_with_default("Client server address", &default_server_addr)?
    } else {
        default_server_addr
    };
    let default_remote_dir = "~/.projector".to_owned();
    let remote_dir = if let Some(remote_dir) = args.remote_dir.clone() {
        remote_dir
    } else if interactive {
        prompt_with_default("Remote install dir", &default_remote_dir)?
    } else {
        default_remote_dir
    };
    let default_sqlite_path = format!("{}/projector.sqlite3", remote_dir.trim_end_matches('/'));
    let sqlite_path = if let Some(sqlite_path) = args.sqlite_path.clone() {
        sqlite_path
    } else if interactive {
        prompt_with_default("Remote SQLite path", &default_sqlite_path)?
    } else {
        default_sqlite_path
    };

    Ok(DeployArgs {
        profile_id: Some(
            args.profile_id
                .clone()
                .ok_or("deploy requires --profile or an interactive terminal")?,
        ),
        ssh_target: Some(ssh_target),
        server_addr: Some(server_addr),
        remote_dir: Some(remote_dir),
        sqlite_path: Some(sqlite_path),
        listen_addr: Some(listen_addr),
        yes: args.yes,
    })
}
