use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(name = "agent-cli", about = "CLI for the NEAR AI agent hosting platform")]
struct Cli {
    /// API server URL (e.g. http://136.119.211.246:30080)
    #[arg(long, env = "AGENT_API_URL", default_value = "http://localhost:8080")]
    api_url: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check API server health
    Health,

    /// Create a new agent instance
    Create {
        /// Instance name
        name: String,
        /// Container image to run
        image: String,
        /// CPU allocation (e.g. "1", "2")
        #[arg(long)]
        cpu: Option<String>,
        /// Memory allocation (e.g. "4Gi", "8Gi")
        #[arg(long)]
        memory: Option<String>,
        /// Disk size (e.g. "10Gi", "50Gi")
        #[arg(long)]
        disk: Option<String>,
        /// Mount path for persistent volume (default: /home/agent)
        #[arg(long)]
        volume_mount: Option<String>,
        /// Security profile: "restricted" (default) or "trusted"
        #[arg(long, default_value = "restricted")]
        security_profile: String,
        /// Ports to expose (NAME:PORT), can be repeated (e.g. --port ssh:22 --port http:8080)
        #[arg(long = "port", value_name = "NAME:PORT")]
        ports: Vec<String>,
        /// Environment variables (KEY=VALUE), can be repeated
        #[arg(long = "env", value_name = "KEY=VALUE")]
        envs: Vec<String>,
    },

    /// List all agent instances
    List,

    /// Get details of a specific instance
    Get {
        /// Instance name
        name: String,
    },

    /// Delete an instance
    Delete {
        /// Instance name
        name: String,
    },

    /// Start a stopped instance
    Start {
        /// Instance name
        name: String,
    },

    /// Stop a running instance
    Stop {
        /// Instance name
        name: String,
    },

    /// Restart an instance
    Restart {
        /// Instance name
        name: String,
    },

    /// Tail logs from an instance
    Logs {
        /// Instance name
        name: String,
        /// Number of lines to return
        #[arg(long, default_value = "100")]
        tail: i64,
    },
}

#[derive(Serialize)]
struct CreateRequest {
    name: String,
    image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cpu: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    volume_mount: Option<String>,
    security_profile: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<PortSpec>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    env: Vec<EnvVar>,
}

#[derive(Serialize)]
struct EnvVar {
    name: String,
    value: String,
}

#[derive(Serialize)]
struct PortSpec {
    name: String,
    port: i32,
}

fn parse_env(s: &str) -> Option<EnvVar> {
    let (name, value) = s.split_once('=')?;
    Some(EnvVar {
        name: name.to_string(),
        value: value.to_string(),
    })
}

fn parse_port(s: &str) -> Option<PortSpec> {
    let (name, port_str) = s.split_once(':')?;
    let port = port_str.parse().ok()?;
    Some(PortSpec {
        name: name.to_string(),
        port,
    })
}

#[derive(Deserialize)]
struct Instance {
    name: String,
    image: String,
    cpu: String,
    memory: String,
    disk: String,
    state: String,
    phase: Option<String>,
    pod_ip: Option<String>,
    host_node: Option<String>,
    restart_count: Option<i32>,
    #[serde(default)]
    node_ports: Vec<NodePortInfo>,
}

#[derive(Deserialize)]
struct NodePortInfo {
    name: String,
    port: i32,
    node_port: i32,
}

#[derive(Deserialize)]
struct LogsResponse {
    name: String,
    lines: Vec<String>,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

fn print_instance(inst: &Instance) {
    println!("Name:          {}", inst.name);
    println!("Image:         {}", inst.image);
    println!("State:         {}", inst.state);
    println!("Phase:         {}", inst.phase.as_deref().unwrap_or("-"));
    println!("CPU:           {}", inst.cpu);
    println!("Memory:        {}", inst.memory);
    println!("Disk:          {}", inst.disk);
    println!("Pod IP:        {}", inst.pod_ip.as_deref().unwrap_or("-"));
    println!("Host Node:     {}", inst.host_node.as_deref().unwrap_or("-"));
    println!(
        "Restart Count: {}",
        inst.restart_count.map_or("-".to_string(), |c| c.to_string())
    );
    if !inst.node_ports.is_empty() {
        println!("Ports:");
        for np in &inst.node_ports {
            println!("  {}:{} → NodePort {}", np.name, np.port, np.node_port);
        }
    }
}

fn print_instance_row(inst: &Instance) {
    println!(
        "{:<20} {:<45} {:<10} {:<15} {:<6} {:<8} {:<8}",
        inst.name,
        inst.image,
        inst.state,
        inst.phase.as_deref().unwrap_or("-"),
        inst.cpu,
        inst.memory,
        inst.disk,
    );
}

async fn handle_error(resp: reqwest::Response) -> String {
    let status = resp.status();
    if let Ok(err) = resp.json::<ErrorResponse>().await {
        format!("Error ({}): {}", status, err.error.message)
    } else {
        format!("Error: {}", status)
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = Client::new();
    let base = cli.api_url.trim_end_matches('/');

    match cli.command {
        Command::Health => {
            let resp = client.get(format!("{base}/health")).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let body: serde_json::Value = r.json().await.unwrap();
                    println!("{}", serde_json::to_string_pretty(&body).unwrap());
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Create {
            name,
            image,
            cpu,
            memory,
            disk,
            volume_mount,
            security_profile,
            ports,
            envs,
        } => {
            let env: Vec<EnvVar> = envs
                .iter()
                .filter_map(|s| parse_env(s))
                .collect();
            let ports: Vec<PortSpec> = ports
                .iter()
                .filter_map(|s| parse_port(s))
                .collect();
            let req = CreateRequest {
                name,
                image,
                cpu,
                memory,
                disk,
                volume_mount,
                security_profile,
                ports,
                env,
            };
            let resp = client
                .post(format!("{base}/instances"))
                .json(&req)
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let inst: Instance = r.json().await.unwrap();
                    println!("Created instance:");
                    print_instance(&inst);
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::List => {
            let resp = client.get(format!("{base}/instances")).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let instances: Vec<Instance> = r.json().await.unwrap();
                    if instances.is_empty() {
                        println!("No instances found.");
                    } else {
                        println!(
                            "{:<20} {:<45} {:<10} {:<15} {:<6} {:<8} {:<8}",
                            "NAME", "IMAGE", "STATE", "PHASE", "CPU", "MEMORY", "DISK"
                        );
                        for inst in &instances {
                            print_instance_row(inst);
                        }
                    }
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Get { name } => {
            let resp = client
                .get(format!("{base}/instances/{name}"))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let inst: Instance = r.json().await.unwrap();
                    print_instance(&inst);
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Delete { name } => {
            let resp = client
                .delete(format!("{base}/instances/{name}"))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    println!("Deleted instance '{name}'.");
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Start { name } => {
            let resp = client
                .post(format!("{base}/instances/{name}/start"))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let inst: Instance = r.json().await.unwrap();
                    println!("Started instance:");
                    print_instance(&inst);
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Stop { name } => {
            let resp = client
                .post(format!("{base}/instances/{name}/stop"))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let inst: Instance = r.json().await.unwrap();
                    println!("Stopped instance:");
                    print_instance(&inst);
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Restart { name } => {
            let resp = client
                .post(format!("{base}/instances/{name}/restart"))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let inst: Instance = r.json().await.unwrap();
                    println!("Restarted instance:");
                    print_instance(&inst);
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }

        Command::Logs { name, tail } => {
            let resp = client
                .get(format!("{base}/instances/{name}/logs?tail={tail}"))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let logs: LogsResponse = r.json().await.unwrap();
                    if logs.lines.is_empty() {
                        println!("No logs for '{}'.", logs.name);
                    } else {
                        for line in &logs.lines {
                            println!("{line}");
                        }
                    }
                }
                Ok(r) => eprintln!("{}", handle_error(r).await),
                Err(e) => eprintln!("Connection failed: {e}"),
            }
        }
    }
}
