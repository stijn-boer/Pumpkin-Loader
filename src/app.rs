use crate::{
    build,
    cli::{Action, Cli, ModAction},
    config::{self, Config},
    error::{LoaderError, Result},
    layout::Layout,
    logging, modding, runtime, source,
};
use clap::Parser;
use std::{fs, path::Path};

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.verbose, cli.quiet);

    if let Action::Init { force } = &cli.command {
        return init(&cli.config, *force);
    }
    
    let config = config::load(&cli.config)?;
    let layout = Layout::new(&cli.config, &config)?;
    layout.create()?;

    match cli.command {
        Action::Init { .. } => unreachable!(),
        Action::Fetch => println!("{}", source::fetch(&config, &layout)?),
        Action::Build { force } => {
            println!("{}", build::ensure(&config, &layout, force)?.display())
        }
        Action::Run { args } => {
            let artifact = build::ensure(&config, &layout, false)?;
            runtime::run(&config, &layout, &artifact, &args)?;
        }
        Action::Status => status(&config, &layout)?,
        Action::Clean => source::clean(&layout)?,
        Action::Mod { command } => match command {
            ModAction::Init(args) => println!(
                "{}",
                modding::init(&config, &layout, &args.name, args.force)?.display()
            ),
            ModAction::Dev(args) => println!(
                "{}",
                modding::prepare_dev(&config, &layout, &args.name)?.display()
            ),
            ModAction::Patch(args) => println!(
                "{}",
                modding::create_patch(&config, &layout, &args.name, &args.patch_name)?.display()
            ),
        },
    }
    Ok(())
}

fn init(config_path: &Path, force: bool) -> Result<()> {
    if config_path.exists() && !force {
        return Err(LoaderError::ConfigAlreadyExists {
            path: config_path.to_path_buf(),
        });
    }
    if let Some(parent) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(config_path, toml::to_string_pretty(&Config::default())?)?;
    log::info!("Created {}", config_path.display());
    log::warn!("Pin pumpkin.revision to a full commit before distributing a modpack");
    Ok(())
}

fn status(config: &Config, layout: &Layout) -> Result<()> {
    println!("repository:  {}", config.pumpkin.repository);
    println!("revision:    {}", config.pumpkin.revision);
    println!("state:       {}", layout.state.display());
    println!("server data: {}", layout.server_data.display());
    match source::resolve_cached_commit(config, layout) {
        Ok(commit) => {
            println!("commit:      {commit}");
            let mods = modding::resolve_all(config, layout, &commit)?;
            println!(
                "build key:   {}",
                build::calculate_key(config, layout, &commit, &mods)?
            );
        }
        Err(_) => println!("commit:      not fetched"),
    }
    Ok(())
}
