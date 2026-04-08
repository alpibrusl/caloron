mod config;
mod daemon;
mod dag;
mod dashboard;
mod git;
mod agent;
mod supervisor;
mod retro;
mod noether;
mod nix;
mod kickoff;

use std::path::Path;

use clap::{Parser, Subcommand};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser)]
#[command(name = "caloron", version, about = "Multi-agent orchestration platform")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start interactive kickoff with PO Agent
    Kickoff {
        /// Sprint goal description
        goal: String,
    },
    /// Start the daemon with an existing DAG
    Start {
        /// Path to dag.json
        #[arg(long, default_value = "dag.json")]
        dag: String,
    },
    /// Gracefully stop the current sprint
    Stop,
    /// Show current sprint state and agent health
    Status,
    /// Show cross-project dashboard (all registered projects)
    Dashboard,
    /// Tail logs for a specific agent role
    Logs {
        /// Agent role to show logs for
        role: String,
    },
    /// Show full event history for a task
    Trace {
        /// Task ID to trace
        task_id: String,
    },
    /// Run retro for the completed sprint
    Retro {
        /// Specific sprint ID (default: current)
        #[arg(long)]
        sprint_id: Option<String>,
    },
    /// Agent management commands
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
}

#[derive(Subcommand)]
enum AgentCommand {
    /// List available agent definitions
    List,
    /// Validate an agent definition YAML
    Validate {
        /// Path to agent YAML file
        file: String,
    },
    /// Build the Nix environment for an agent
    Build {
        /// Path to agent YAML file
        file: String,
    },
    /// Generate an agent definition from a spec
    Generate {
        /// Agent spec YAML file (or use --interactive)
        #[arg(long)]
        spec: Option<String>,
        /// Personality (developer, qa, reviewer, architect, designer, ux-researcher, devops)
        #[arg(long, short = 'p')]
        personality: Option<String>,
        /// Capabilities (comma-separated: code-writing,testing,rust,nodejs,python,frontend,browser-research,noether)
        #[arg(long, short = 'c')]
        capabilities: Option<String>,
        /// Model (balanced, strong, fast, gemini-pro, gemini-flash)
        #[arg(long, short = 'm')]
        model: Option<String>,
        /// Framework (claude-code, gemini-cli, aider, codex-cli)
        #[arg(long, short = 'f')]
        framework: Option<String>,
        /// Agent instance name
        #[arg(long)]
        name: Option<String>,
        /// Output file path (default: stdout)
        #[arg(long, short = 'o')]
        output: Option<String>,
        /// List available options for each axis
        #[arg(long)]
        list_options: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(
            EnvFilter::try_from_env("CALORON_LOG_LEVEL").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Command::Kickoff { goal } => {
            let repo_root = std::env::current_dir()?;

            // Step 1: Read repository state
            println!("Analyzing repository...\n");
            let repo_state = kickoff::po_agent::read_repository_state(&repo_root)?;
            println!("{}", repo_state.to_context_string());

            // Step 2: Show PO Agent prompt context
            println!("---");
            println!("Sprint goal: {goal}");
            println!("---\n");
            println!("The PO Agent would now engage in dialogue to refine the sprint scope.");
            println!("For now, provide a DAG JSON file path or paste DAG JSON:\n");

            // Step 3: Read DAG from stdin or file
            // In production, this would be an interactive LLM session.
            // For Phase 5, we accept a DAG file path as input.
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim();

            let dag = if Path::new(input).exists() {
                let content = std::fs::read_to_string(input)?;
                serde_json::from_str(&content)?
            } else {
                // Try to parse as inline JSON
                match kickoff::po_agent::extract_dag_from_output(input) {
                    Some(dag) => dag,
                    None => {
                        eprintln!("Could not parse DAG from input. Provide a path to a dag.json file.");
                        std::process::exit(1);
                    }
                }
            };

            // Step 4: Validate and summarize
            let engine = dag::engine::DagEngine::from_dag(dag.clone())?;
            println!("\n{}", kickoff::po_agent::summarize_dag(&dag));

            println!("\nProceed? [y/N]");
            let mut confirm = String::new();
            std::io::stdin().read_line(&mut confirm)?;
            if !confirm.trim().eq_ignore_ascii_case("y") {
                println!("Kickoff cancelled.");
                return Ok(());
            }

            // Step 5: Write DAG
            let dag_path = Path::new("dag.json");
            kickoff::po_agent::write_dag_to_file(&dag, dag_path)?;
            println!("DAG written to {}", dag_path.display());

            // Step 6: Create issues (requires GitHub connection)
            let config_path = Path::new("caloron.toml");
            if config_path.exists() {
                let cfg = config::load_config(config_path)?;
                let token = std::env::var(&cfg.github.token_env).ok();

                if let Some(token) = token {
                    let (owner, repo) = cfg.project.repo.split_once('/').unwrap_or(("", ""));
                    if !owner.is_empty() {
                        let client = git::GitHubClient::new(&token, owner, repo)?;
                        println!("Creating issues...");
                        let created = kickoff::issue_creator::IssueCreator::create_all(&client, &dag).await?;
                        for (task_id, issue_num) in &created {
                            println!("  {task_id} → #{issue_num}");
                        }
                        println!("\nSprint ready. Run `caloron start` to begin.");
                    }
                } else {
                    println!("No GitHub token — skipping issue creation.");
                    println!("Run `caloron start --dag dag.json` to begin.");
                }
            } else {
                println!("No caloron.toml — skipping issue creation.");
                println!("Run `caloron start --dag dag.json` to begin.");
            }

            Ok(())
        }
        Command::Start { dag } => {
            let config_path = Path::new("caloron.toml");
            let dag_path = Path::new(&dag);
            daemon::orchestrator::start_daemon(config_path, dag_path).await
        }
        Command::Stop => {
            tracing::info!("Stopping sprint");
            todo!("stop not yet implemented")
        }
        Command::Status => {
            let state_files: Vec<_> = std::fs::read_dir("state")
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                .collect();

            if state_files.is_empty() {
                println!("No active sprint found. Use `caloron start` to begin.");
                return Ok(());
            }

            let latest = state_files
                .iter()
                .max_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
                .unwrap();

            let engine = dag::engine::DagEngine::resume_from_file(&latest.path())?;
            dashboard::print_project_status(engine.state());

            Ok(())
        }
        Command::Dashboard => {
            dashboard::print_dashboard()?;
            Ok(())
        }
        Command::Logs { role } => {
            tracing::info!(role, "Tailing logs");
            todo!("logs not yet implemented")
        }
        Command::Trace { task_id } => {
            tracing::info!(task_id, "Tracing task");
            todo!("trace not yet implemented")
        }
        Command::Retro { sprint_id } => {
            // Find the sprint state file
            let state_path = if let Some(id) = &sprint_id {
                format!("state/sprint-{id}.json")
            } else {
                // Find most recent
                let mut files: Vec<_> = std::fs::read_dir("state")
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                    .collect();
                files.sort_by_key(|e| e.metadata().ok().and_then(|m| m.modified().ok()));
                match files.last() {
                    Some(f) => f.path().to_string_lossy().to_string(),
                    None => {
                        eprintln!("No sprint state files found in state/");
                        std::process::exit(1);
                    }
                }
            };

            let engine = dag::engine::DagEngine::resume_from_file(Path::new(&state_path))?;
            let dag_state = engine.state();

            // For now, create sample feedback from what we have.
            // In production, feedback would be collected from GitHub comments during the sprint.
            println!("Retro for sprint: {}", dag_state.sprint.id);
            println!("Loading feedback from sprint state...\n");

            // Build empty feedback (no comments collected yet — Phase 4 stores them)
            let feedback = retro::collector::SprintFeedback::from_feedbacks(
                &dag_state.sprint.id,
                vec![], // TODO: load from stored feedback buffer
                dag_state,
            );

            let analysis = retro::analyzer::analyze(&feedback);
            let report = retro::report::generate_report(&feedback, &analysis);

            // Write report
            let report_path = format!("retro/sprint-{}.md", dag_state.sprint.id);
            retro::report::write_report(&report, Path::new(&report_path))?;

            println!("{report}");
            println!("Report written to {report_path}");

            Ok(())
        }
        Command::Agent { command } => match command {
            AgentCommand::List => {
                todo!("agent list not yet implemented")
            }
            AgentCommand::Validate { file } => {
                let config_path = Path::new("caloron.toml");
                let cfg = if config_path.exists() {
                    config::load_config(config_path)?
                } else {
                    // Minimal default config for validation without a config file
                    toml::from_str("[project]\nname = \"\"\nrepo = \"\"\nmeta_repo = \"\"\n[github]\n[llm]\n")?
                };

                let (def, result) = agent::definition::load_and_validate(Path::new(&file), &cfg)?;
                agent::definition::print_validation(&def, &result);

                if !result.is_valid() {
                    std::process::exit(1);
                }
                Ok(())
            }
            AgentCommand::Build { file } => {
                let config_path = Path::new("caloron.toml");
                let cfg = if config_path.exists() {
                    config::load_config(config_path)?
                } else {
                    toml::from_str("[project]\nname = \"\"\nrepo = \"\"\nmeta_repo = \"\"\n[github]\n[llm]\n")?
                };

                let (def, result) = agent::definition::load_and_validate(Path::new(&file), &cfg)?;
                if !result.is_valid() {
                    agent::definition::print_validation(&def, &result);
                    std::process::exit(1);
                }

                let params = nix::NixGenerator::default_params(&def.name);

                if cfg.nix.enabled {
                    // Build real Nix environment
                    let caloron_dir = std::env::current_dir()?.join(".caloron");
                    let builder = nix::NixBuilder::new(&caloron_dir, true);

                    println!("Building Nix environment for agent: {}", def.name);
                    let env = builder.build_env(&def, &params).await?;
                    println!("Nix environment built successfully.");
                    println!("  Flake: {}", env.flake_path.display());
                    println!("  Shell: nix develop .#{}", env.shell_attr);

                    // Print the generated flake for reference
                    let flake_path = env.flake_path.join("flake.nix");
                    if flake_path.exists() {
                        println!("\nGenerated flake.nix:");
                        println!("{}", std::fs::read_to_string(&flake_path)?);
                    }
                } else {
                    // Just print the expression
                    println!("Nix is disabled. Generated devShell expression:\n");
                    let nix_expr = nix::NixGenerator::generate_devshell(&def, &params);
                    println!("{nix_expr}");
                    println!("Enable Nix in caloron.toml to build the environment.");
                }

                Ok(())
            }
            AgentCommand::Generate {
                spec,
                personality,
                capabilities,
                model,
                framework,
                name,
                output,
                list_options,
            } => {
                let registry = agent::registry::default_registry();

                if list_options {
                    println!("Personalities:");
                    for (k, v) in &registry.personalities {
                        println!("  {k:<20} {}", v.description);
                    }
                    println!("\nCapabilities:");
                    for (k, v) in &registry.capabilities {
                        println!("  {k:<20} {}", v.description);
                    }
                    println!("\nModels:");
                    for (k, v) in &registry.models {
                        println!("  {k:<20} {} (temp: {}, tokens: {})", v.model, v.temperature, v.max_tokens);
                    }
                    println!("\nFrameworks:");
                    for (k, v) in &registry.frameworks {
                        println!("  {k:<20} {} [{}]", v.description, v.command);
                    }
                    return Ok(());
                }

                let agent_spec = if let Some(spec_path) = spec {
                    let content = std::fs::read_to_string(&spec_path)?;
                    serde_yaml::from_str(&content)?
                } else {
                    // Build spec from CLI flags
                    let p = personality.unwrap_or_else(|| {
                        eprintln!("--personality is required (or use --spec)");
                        std::process::exit(1);
                    });
                    let caps: Vec<String> = capabilities
                        .unwrap_or_else(|| "code-writing".into())
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect();

                    caloron_types::agent_gen::AgentSpec {
                        name: name.unwrap_or_else(|| format!("{p}-1")),
                        personality: p,
                        capabilities: caps,
                        model: model.unwrap_or_else(|| "balanced".into()),
                        framework: framework.unwrap_or_else(|| "claude-code".into()),
                        extra_instructions: None,
                        overrides: caloron_types::agent_gen::AgentOverrides::default(),
                    }
                };

                let def = registry.generate(&agent_spec).map_err(|e| anyhow::anyhow!(e))?;

                let yaml = serde_yaml::to_string(&def)?;

                if let Some(out_path) = output {
                    std::fs::write(&out_path, &yaml)?;
                    println!("Generated agent definition written to {out_path}");
                } else {
                    println!("{yaml}");
                }

                Ok(())
            }
        },
    }
}
