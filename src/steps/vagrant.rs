use crate::execution_context::ExecutionContext;
use crate::executor::CommandExt;
use crate::terminal::print_separator;
use crate::utils;
use anyhow::Result;
use log::debug;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fmt::Display, str::FromStr};
use strum::EnumString;

#[derive(Debug, Copy, Clone, EnumString)]
#[strum(serialize_all = "lowercase")]
enum BoxStatus {
    PowerOff,
    Running,
    Saved,
    Aborted,
}

impl BoxStatus {
    fn powered_on(self) -> bool {
        match self {
            BoxStatus::Running => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
struct VagrantBox<'a> {
    path: &'a str,
    name: String,
}

impl<'a> Display for VagrantBox<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} @ {}", self.name, self.path)
    }
}

struct Vagrant {
    path: PathBuf,
}

impl Vagrant {
    fn get_boxes<'a>(&self, directory: &'a str) -> Result<Vec<(VagrantBox<'a>, BoxStatus)>> {
        let output = Command::new(&self.path)
            .arg("status")
            .current_dir(directory)
            .check_output()?;
        debug!("Vagrant output in {}: {}", directory, output);

        let boxes = output
            .split('\n')
            .skip(2)
            .take_while(|line| !(line.is_empty() || line.starts_with('\r')))
            .map(|line| {
                debug!("Vagrant line: {:?}", line);
                let mut elements = line.split_whitespace();
                let vagrant_box = VagrantBox {
                    name: elements.next().unwrap().to_string(),
                    path: directory,
                };
                let box_status = BoxStatus::from_str(elements.next().unwrap()).unwrap();
                debug!("{:?}: {:?}", vagrant_box, box_status);
                (vagrant_box, box_status)
            })
            .collect();

        Ok(boxes)
    }

    fn temporary_power_on<'a>(
        &'a self,
        vagrant_box: &'a VagrantBox,
        status: BoxStatus,
        ctx: &'a ExecutionContext,
    ) -> Result<TemporaryPowerOn<'a>> {
        TemporaryPowerOn::create(&self.path, vagrant_box, status, ctx)
    }
}

struct TemporaryPowerOn<'a> {
    vagrant: &'a Path,
    vagrant_box: &'a VagrantBox<'a>,
    status: BoxStatus,
    ctx: &'a ExecutionContext<'a>,
}

impl<'a> TemporaryPowerOn<'a> {
    fn create(
        vagrant: &'a Path,
        vagrant_box: &'a VagrantBox<'a>,
        status: BoxStatus,
        ctx: &'a ExecutionContext<'a>,
    ) -> Result<Self> {
        let subcommand = match status {
            BoxStatus::PowerOff | BoxStatus::Aborted => "up",
            BoxStatus::Saved => "resume",
            BoxStatus::Running => unreachable!(),
        };
        println!("Powering on {}", vagrant_box);

        ctx.run_type()
            .execute(vagrant)
            .args(&[subcommand, &vagrant_box.name])
            .current_dir(vagrant_box.path)
            .check_run()?;
        Ok(TemporaryPowerOn {
            vagrant,
            vagrant_box,
            status,
            ctx,
        })
    }
}

impl<'a> Drop for TemporaryPowerOn<'a> {
    fn drop(&mut self) {
        let subcommand = match self.status {
            BoxStatus::PowerOff | BoxStatus::Aborted => "halt",
            BoxStatus::Saved => "suspend",
            BoxStatus::Running => unreachable!(),
        };

        println!("Powering off {}", self.vagrant_box);
        self.ctx
            .run_type()
            .execute(self.vagrant)
            .args(&[subcommand, &self.vagrant_box.name])
            .current_dir(self.vagrant_box.path)
            .check_run()
            .ok();
    }
}

pub fn topgrade_vagrant_boxes(ctx: &ExecutionContext) -> Result<()> {
    let directories = utils::require_option(ctx.config().vagrant_directories())?;
    let vagrant = Vagrant {
        path: utils::require("vagrant")?,
    };

    print_separator("Vagrant");

    for directory in directories {
        let boxes = vagrant.get_boxes(directory)?;
        debug!("{:?}", boxes);
        for (vagrant_box, status) in boxes {
            let mut _poweron = None;
            if !status.powered_on() {
                if !(ctx.config().vagrant_power_on().unwrap_or(true)) {
                    debug!("Skipping powered off box {}", vagrant_box);
                    continue;
                } else {
                    _poweron = Some(vagrant.temporary_power_on(&vagrant_box, status, ctx)?);
                }
            }

            println!("Running Topgrade in {}", vagrant_box);
            let pathbuf = PathBuf::from(directory);
            let prefix = if vagrant_box.name == "default" {
                pathbuf.file_name().unwrap().to_str().unwrap()
            } else {
                &vagrant_box.name
            };

            let mut command = format!("env TOPGRADE_PREFIX={} topgrade", prefix);
            if ctx.config().yes() {
                command.push_str(" -y");
            }

            ctx.run_type()
                .execute(&vagrant.path)
                .current_dir(directory)
                .args(&["ssh", "-c", &command])
                .check_run()?;
        }
    }
    Ok(())
}