use std::{fs::{self}, path::{Path, PathBuf}, os::unix::fs::PermissionsExt };

use anyhow::{Result, anyhow};

use nix::unistd::Pid;
use oci_spec::LinuxResources;

use crate::{cgroups::v2::controller::Controller, cgroups::{common::{CgroupManager, self}, v2::controller_type::ControllerType}, utils::PathBufExt};
use super::{cpu::Cpu, cpuset::CpuSet, memory::Memory};

const CGROUP_PROCS: &str = "cgroup.procs";
const CGROUP_CONTROLLERS: &str = "cgroup.controllers";
const CGROUP_SUBTREE_CONTROL: &str = "cgroup.subtree_control";

const CONTROLLER_TYPES: &[ControllerType] = &[ControllerType::Cpu, ControllerType::CpuSet, ControllerType::Memory];

pub struct Manager {
   root_path: PathBuf,
   cgroup_path: PathBuf,
}

impl Manager {
    pub fn new(root_path: PathBuf, cgroup_path: PathBuf) -> Result<Self> {
        Ok(Self {
            root_path,
            cgroup_path,
        })
    }

    pub fn remove(&self, cgroup_path: &Path) -> Result<()> {
        let full_path = self.root_path.join(cgroup_path);
        fs::remove_dir_all(full_path)?;

        Ok(())
    }

    fn create_unified_cgroup(&self, cgroup_path: &Path, pid: Pid) -> Result<PathBuf> {
        let full_path = self.root_path.join_absolute_path(cgroup_path)?;  
        let controllers: Vec<String> = self.get_available_controllers(
        common::DEFAULT_CGROUP_ROOT)?.into_iter().map(|c|format!("{}{}","+", c.to_string())).collect();

        let mut current_path = self.root_path.clone();
        let mut components = cgroup_path.components().skip(1).peekable();
        while let Some(component) = components.next() {
            current_path = current_path.join(component);
            if !current_path.exists() {
                fs::create_dir(&current_path)?;
                fs::metadata(&current_path)?.permissions().set_mode(0o755);
            }

            // last component cannot have subtree_control enabled due to internal process constraint
            // if this were set, writing to the cgroups.procs file will fail with Erno 16 (device or resource busy)
            if components.peek().is_some() {
                for controller in &controllers {
                    common::write_cgroup_file(&current_path.join(CGROUP_SUBTREE_CONTROL), controller)?;
                }
            }
        }

        common::write_cgroup_file(&full_path.join(CGROUP_PROCS), &pid.to_string())?;       
        Ok(full_path)
    }

    fn get_available_controllers<P: AsRef<Path>>(&self, cgroup_path: P) -> Result<Vec<ControllerType>> {
        let controllers_path = self.root_path.join(cgroup_path).join(CGROUP_CONTROLLERS);
        if !controllers_path.exists() {
            return Err(anyhow!("cannot get available controllers. {:?} does not exist", controllers_path));
        }
        
        let mut controllers = Vec::new();
        for controller in fs::read_to_string(&controllers_path)?.split_whitespace() {
            match controller {
                "cpu" => controllers.push(ControllerType::Cpu),
                "cpuset" => controllers.push(ControllerType::CpuSet),
                "io" => controllers.push(ControllerType::IO),
                "memory" => controllers.push(ControllerType::Memory),
                "pids" => controllers.push(ControllerType::Pids),
                "hugetlb" => controllers.push(ControllerType::HugeTlb),
                _ => continue,
            }
        }

        Ok(controllers)
    }
}

impl CgroupManager for Manager {
    fn apply(&self, linux_resources: &LinuxResources, pid: Pid) -> Result<()> {
        let cgroup_path = self.create_unified_cgroup(&self.cgroup_path, pid)?;

        for controller in CONTROLLER_TYPES {
            match controller {
                ControllerType::Cpu => Cpu::apply(linux_resources, &self.root_path)?,
                ControllerType::CpuSet => CpuSet::apply(linux_resources, &cgroup_path)?,
                ControllerType::Memory => Memory::apply(linux_resources, &self.root_path)?,
                _ => continue,
            }
        }

        Ok(())
    }
}

