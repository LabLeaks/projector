/**
@module PROJECTOR.EDGE.CONNECTION_ARGS
Owns parsing and typed argument shapes for connect, disconnect, and deploy edge commands.
*/
// @fileimplements PROJECTOR.EDGE.CONNECTION_ARGS
use std::error::Error;

#[derive(Clone, Debug)]
pub(super) struct ConnectArgs {
    pub(super) profile_id: Option<String>,
    pub(super) server_addr: Option<String>,
    pub(super) ssh_target: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct DisconnectArgs {
    pub(super) profile_id: String,
    pub(super) yes: bool,
}

#[derive(Clone, Debug)]
pub(super) struct DeployArgs {
    pub(super) profile_id: Option<String>,
    pub(super) ssh_target: Option<String>,
    pub(super) server_addr: Option<String>,
    pub(super) remote_dir: Option<String>,
    pub(super) sqlite_path: Option<String>,
    pub(super) listen_addr: Option<String>,
    pub(super) yes: bool,
}

pub(super) fn parse_connect_args(args: &[String]) -> Result<ConnectArgs, Box<dyn Error>> {
    let mut parsed = ConnectArgs {
        profile_id: None,
        server_addr: None,
        ssh_target: None,
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--id" => {
                idx += 1;
                parsed.profile_id = Some(args.get(idx).ok_or("missing value after --id")?.clone());
            }
            "--server" => {
                idx += 1;
                parsed.server_addr =
                    Some(args.get(idx).ok_or("missing value after --server")?.clone());
            }
            "--ssh" => {
                idx += 1;
                parsed.ssh_target = Some(args.get(idx).ok_or("missing value after --ssh")?.clone());
            }
            other => return Err(format!("unexpected connect argument: {other}").into()),
        }
        idx += 1;
    }
    Ok(parsed)
}

pub(super) fn parse_disconnect_args(args: &[String]) -> Result<DisconnectArgs, Box<dyn Error>> {
    let mut yes = false;
    let mut profile_id = None;
    for arg in args {
        if arg == "--yes" {
            yes = true;
        } else if profile_id.is_none() {
            profile_id = Some(arg.clone());
        } else {
            return Err(format!("unexpected extra disconnect argument: {arg}").into());
        }
    }
    Ok(DisconnectArgs {
        profile_id: profile_id.ok_or("disconnect requires a profile id")?,
        yes,
    })
}

pub(super) fn parse_deploy_args(args: &[String]) -> Result<DeployArgs, Box<dyn Error>> {
    let mut parsed = DeployArgs {
        profile_id: None,
        ssh_target: None,
        server_addr: None,
        remote_dir: None,
        sqlite_path: None,
        listen_addr: None,
        yes: false,
    };
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--profile" => {
                idx += 1;
                parsed.profile_id = Some(
                    args.get(idx)
                        .ok_or("missing value after --profile")?
                        .clone(),
                );
            }
            "--ssh" => {
                idx += 1;
                parsed.ssh_target = Some(args.get(idx).ok_or("missing value after --ssh")?.clone());
            }
            "--server-addr" => {
                idx += 1;
                parsed.server_addr = Some(
                    args.get(idx)
                        .ok_or("missing value after --server-addr")?
                        .clone(),
                );
            }
            "--remote-dir" => {
                idx += 1;
                parsed.remote_dir = Some(
                    args.get(idx)
                        .ok_or("missing value after --remote-dir")?
                        .clone(),
                );
            }
            "--sqlite-path" => {
                idx += 1;
                parsed.sqlite_path = Some(
                    args.get(idx)
                        .ok_or("missing value after --sqlite-path")?
                        .clone(),
                );
            }
            "--listen-addr" => {
                idx += 1;
                parsed.listen_addr = Some(
                    args.get(idx)
                        .ok_or("missing value after --listen-addr")?
                        .clone(),
                );
            }
            "--yes" => {
                parsed.yes = true;
            }
            other => return Err(format!("unexpected deploy argument: {other}").into()),
        }
        idx += 1;
    }
    Ok(parsed)
}
