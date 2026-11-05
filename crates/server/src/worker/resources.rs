use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    net::IpAddr,
    path::{Path, PathBuf},
};

fn preferred_addresses(values: impl IntoIterator<Item = IpAddr>) -> Vec<String> {
    let valid = values
        .into_iter()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast())
        .collect::<BTreeSet<_>>();
    let ipv4 = valid
        .iter()
        .filter(|ip| ip.is_ipv4())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if !ipv4.is_empty() {
        ipv4
    } else {
        valid
            .iter()
            .filter(|ip| !matches!(ip, IpAddr::V6(v) if v.is_unicast_link_local()))
            .map(ToString::to_string)
            .collect()
    }
}

fn path_within(data: &Path, mount: &Path) -> bool {
    #[cfg(windows)]
    fn normalize(path: &Path) -> PathBuf {
        PathBuf::from(
            path.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .replace('/', "\\")
                .to_lowercase(),
        )
    }
    #[cfg(not(windows))]
    fn normalize(path: &Path) -> PathBuf {
        path.to_path_buf()
    }
    normalize(data).starts_with(normalize(mount))
}

pub fn sample(data: PathBuf) -> (Value, Vec<String>) {
    let mut system = sysinfo::System::new();
    system.refresh_cpu_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_cpu_all();
    system.refresh_memory();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let canonical = data.canonicalize().unwrap_or(data);
    let selected = disks
        .list()
        .iter()
        .filter(|d| path_within(&canonical, d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    let partitions = disks.list().iter().map(|disk| {
        let total = disk.total_space();
        let available = disk.available_space();
        let kind = match disk.kind() {
            sysinfo::DiskKind::SSD => "ssd",
            sysinfo::DiskKind::HDD => "hdd",
            _ => "unknown",
        };
        json!({"name":disk.name().to_string_lossy(),"mount":disk.mount_point().display().to_string(),"filesystem":disk.file_system().to_string_lossy(),"kind":kind,"total":total,"available":available,"used":total.saturating_sub(available),"usage_percent":if total>0 {Some(100.0*(total.saturating_sub(available)) as f64/total as f64)}else{None},"data_disk":selected.is_some_and(|d|d.mount_point()==disk.mount_point())})
    }).collect::<Vec<_>>();
    (
        json!({"cpu_cores":system.cpus().len(),"cpu_logical_processors":system.cpus().len(),"cpu_physical_cores":sysinfo::System::physical_core_count(),"cpu_packages":cpu_packages(),"cpu_model":system.cpus().first().map(|c|c.brand()),"cpu_percent":system.global_cpu_usage(),"memory_total":system.total_memory(),"memory_available":system.available_memory(),"memory_used":system.used_memory(),"disk_total":selected.map(|d|d.total_space()),"disk_available":selected.map(|d|d.available_space()),"disk_mount":selected.map(|d|d.mount_point().display().to_string()),"disk_count":partitions.len(),"disks":partitions,"sampled_at":crate::db::now()}),
        preferred_addresses(physical_addresses()),
    )
}

#[cfg(target_os = "linux")]
fn cpu_packages() -> Option<usize> {
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let ids = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.trim() == "physical id")
        .map(|(_, id)| id.trim())
        .collect::<BTreeSet<_>>();
    (!ids.is_empty()).then_some(ids.len())
}
#[cfg(target_os = "macos")]
fn cpu_packages() -> Option<usize> {
    let output = std::process::Command::new("sysctl")
        .args(["-n", "hw.packages"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn cpu_packages() -> Option<usize> {
    None
}

#[cfg(not(windows))]
fn physical_addresses() -> Vec<IpAddr> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    #[cfg(target_os = "macos")]
    let physical = std::process::Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
        .ok()
        .filter(|v| v.status.success())
        .and_then(|v| String::from_utf8(v.stdout).ok())
        .map(|text| {
            text.lines()
                .filter_map(|line| line.strip_prefix("Device: "))
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    networks
        .iter()
        .filter(|(name, _)| {
            #[cfg(target_os = "linux")]
            {
                Path::new("/sys/class/net")
                    .join(name)
                    .join("device")
                    .exists()
            }
            #[cfg(target_os = "macos")]
            {
                physical.contains(*name)
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                let _ = name;
                false
            }
        })
        .flat_map(|(_, network)| network.ip_networks().iter().map(|ip| ip.addr))
        .collect()
}

#[cfg(windows)]
fn cpu_packages() -> Option<usize> {
    use windows_sys::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorPackage,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    let mut bytes = 0;
    // Windows writes variable-sized records; usize backing storage provides native alignment.
    unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorPackage,
            std::ptr::null_mut(),
            &mut bytes,
        );
    }
    if bytes == 0 {
        return None;
    }
    let mut storage = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorPackage,
            storage.as_mut_ptr().cast(),
            &mut bytes,
        )
    } == 0
    {
        return None;
    }
    let mut offset = 0usize;
    let mut count = 0;
    while offset + 8 <= bytes as usize {
        let record = unsafe {
            storage
                .as_ptr()
                .cast::<u8>()
                .add(offset)
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
        };
        let size = unsafe { (*record).Size } as usize;
        if size < 8 || offset + size > bytes as usize {
            return None;
        }
        if unsafe { (*record).Relationship } == RelationProcessorPackage {
            count += 1;
        }
        offset += size;
    }
    (count > 0).then_some(count)
}

#[cfg(windows)]
fn physical_addresses() -> Vec<IpAddr> {
    use windows_sys::Win32::{
        NetworkManagement::IpHelper::{
            GetAdaptersAddresses, GetIfEntry2, IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2,
        },
        Networking::WinSock::{AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6},
    };
    let mut bytes = 0;
    unsafe {
        GetAdaptersAddresses(
            AF_UNSPEC as u32,
            0,
            std::ptr::null(),
            std::ptr::null_mut(),
            &mut bytes,
        );
    }
    if bytes == 0 {
        return vec![];
    }
    let mut storage = vec![0usize; (bytes as usize).div_ceil(std::mem::size_of::<usize>())];
    let first = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
    if unsafe { GetAdaptersAddresses(AF_UNSPEC as u32, 0, std::ptr::null(), first, &mut bytes) }
        != 0
    {
        return vec![];
    }
    let mut result = Vec::new();
    let mut current = first;
    // Pointers are owned by storage and remain valid until enumeration ends.
    unsafe {
        while !current.is_null() {
            let adapter = &*current;
            current = adapter.Next;
            let mut row = MIB_IF_ROW2::default();
            row.InterfaceIndex = adapter.Anonymous1.Anonymous.IfIndex;
            if GetIfEntry2(&mut row) != 0
                || row.InterfaceAndOperStatusFlags._bitfield & 1 == 0
                || row.OperStatus != 1
            {
                continue;
            }
            let mut address = adapter.FirstUnicastAddress;
            while !address.is_null() {
                let item = &*address;
                address = item.Next;
                let socket = item.Address;
                if socket.lpSockaddr.is_null() {
                    continue;
                }
                match (*socket.lpSockaddr).sa_family {
                    AF_INET
                        if socket.iSockaddrLength as usize
                            >= std::mem::size_of::<SOCKADDR_IN>() =>
                    {
                        let ip = &*socket.lpSockaddr.cast::<SOCKADDR_IN>();
                        result.push(IpAddr::V4(ip.sin_addr.S_un.S_addr.to_ne_bytes().into()));
                    }
                    AF_INET6
                        if socket.iSockaddrLength as usize
                            >= std::mem::size_of::<SOCKADDR_IN6>() =>
                    {
                        let ip = &*socket.lpSockaddr.cast::<SOCKADDR_IN6>();
                        result.push(IpAddr::V6(ip.sin6_addr.u.Byte.into()));
                    }
                    _ => {}
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_prefer_ipv4_and_fall_back_to_ipv6() {
        let ips = |values: &[&str]| {
            values
                .iter()
                .map(|v| v.parse::<IpAddr>().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            preferred_addresses(ips(&[
                "127.0.0.1",
                "::1",
                "::",
                "192.168.1.9",
                "2001:db8::1",
                "192.168.1.9"
            ])),
            vec!["192.168.1.9"]
        );
        assert_eq!(
            preferred_addresses(ips(&["fe80::1", "::1", "2001:db8::1"])),
            vec!["2001:db8::1"]
        );
    }
    #[test]
    fn mounted_path_uses_component_boundaries() {
        assert!(path_within(Path::new("/data/ferry"), Path::new("/data")));
        assert!(!path_within(
            Path::new("/database/ferry"),
            Path::new("/data")
        ));
        #[cfg(windows)]
        {
            assert!(path_within(
                Path::new(r"\\?\D:\Projects\ferry"),
                Path::new(r"d:\")
            ));
            assert!(!path_within(
                Path::new(r"\\?\D:\Projects\ferry"),
                Path::new(r"C:\")
            ));
        }
    }
    #[cfg(windows)]
    #[test]
    fn real_windows_sample_finds_the_data_volume_and_cpu_topology() {
        let directory = tempfile::tempdir().unwrap();
        let (sample, addresses) = sample(directory.path().to_path_buf());
        assert!(
            sample["disk_total"].as_u64().is_some_and(|size| size > 0),
            "{sample}"
        );
        let disks = sample["disks"].as_array().unwrap();
        assert!(!disks.is_empty());
        assert_eq!(disks.iter().filter(|d| d["data_disk"] == true).count(), 1);
        for disk in disks {
            assert_eq!(
                disk["total"].as_u64().unwrap(),
                disk["available"].as_u64().unwrap() + disk["used"].as_u64().unwrap()
            );
        }
        assert!(sample["cpu_packages"].as_u64().is_some_and(|n| n > 0));
        assert!(sample["cpu_physical_cores"].as_u64().is_some_and(|n| n > 0));
        assert!(
            sample["cpu_logical_processors"].as_u64().unwrap()
                >= sample["cpu_physical_cores"].as_u64().unwrap()
        );
        let addresses = addresses
            .iter()
            .map(|ip| ip.parse::<IpAddr>().unwrap())
            .collect::<Vec<_>>();
        assert!(addresses.iter().all(IpAddr::is_ipv4) || addresses.iter().all(IpAddr::is_ipv6));
        assert!(
            addresses
                .iter()
                .all(|ip| !ip.is_loopback() && !ip.is_unspecified())
        );
    }
}
