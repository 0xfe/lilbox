//! Small classic-BPF allow-by-default filter, consumed and inherited via bwrap.
//! Reject other ABIs so syscall-number aliases cannot bypass a restriction.
use crate::config::Network;
use anyhow::Result;

pub fn description(network: Network) -> &'static str {
    if network == Network::Deny {
        "native ABI only; deny sockets except private NETLINK_ROUTE (bubblewrap loopback setup), socketpair, namespace creation, mount changes, ptrace/process_vm, kernel keyring, bpf, io_uring, open_by_handle_at, terminal injection; clone3 returns ENOSYS"
    } else {
        "native ABI only; deny namespace creation, mount changes, ptrace/process_vm, kernel keyring, bpf, io_uring, open_by_handle_at, terminal injection; clone3 returns ENOSYS"
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
pub fn filter(_network: Network) -> Result<Vec<u8>> {
    anyhow::bail!("bubblewrap syscall filtering supports only x86_64 and aarch64 Linux")
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub fn filter(network: Network) -> Result<Vec<u8>> {
    const LOAD: u16 = 0x20;
    const EQ: u16 = 0x15;
    const SET: u16 = 0x45;
    const RET: u16 = 0x06;
    const DENY: u32 = 0x0005_0000 | libc::EPERM as u32;
    const KILL: u32 = 0x8000_0000;
    #[cfg(target_arch = "x86_64")]
    let arch = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    let arch = 0xc000_00b7;
    let mut code: Vec<(u16, u8, u8, u32)> = vec![
        (LOAD, 0, 0, 4),
        (EQ, 1, 0, arch),
        (RET, 0, 0, KILL),
        (LOAD, 0, 0, 0),
    ];
    // x32 shares AUDIT_ARCH_X86_64 but uses different syscall numbers.
    #[cfg(target_arch = "x86_64")]
    code.extend([(SET, 0, 1, 0x4000_0000), (RET, 0, 0, KILL)]);
    let mut denied = vec![
        libc::SYS_unshare,
        libc::SYS_setns,
        libc::SYS_mount,
        libc::SYS_umount2,
        libc::SYS_pivot_root,
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_open_by_handle_at,
        libc::SYS_bpf,
        libc::SYS_keyctl,
        libc::SYS_add_key,
        libc::SYS_request_key,
        // io_uring can perform socket/connect operations without passing through
        // the corresponding syscall filter. Do not permit that alternate API.
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        // New mount API, including moving/cloning mounts.
        libc::SYS_open_tree,
        libc::SYS_move_mount,
        libc::SYS_fsopen,
        libc::SYS_fsconfig,
        libc::SYS_fsmount,
        libc::SYS_fspick,
        libc::SYS_mount_setattr,
    ];
    if network == Network::Deny {
        denied.push(libc::SYS_socketpair);
    }
    for syscall in denied {
        code.extend([(EQ, 0, 1, syscall as u32), (RET, 0, 0, DENY)]);
    }
    if network == Network::Deny {
        // bwrap configures loopback after installing the filter. Permit only
        // route netlink in the private network namespace; never TCP/UDP/Unix.
        code.extend([
            (EQ, 0, 7, libc::SYS_socket as u32),
            (LOAD, 0, 0, 16),
            (EQ, 1, 0, libc::AF_NETLINK as u32),
            (RET, 0, 0, DENY),
            (LOAD, 0, 0, 32),
            (EQ, 1, 0, libc::NETLINK_ROUTE as u32),
            (RET, 0, 0, DENY),
            (RET, 0, 0, 0x7fff_0000),
            (LOAD, 0, 0, 0),
        ]);
    }
    code.extend([
        (EQ, 0, 1, libc::SYS_clone3 as u32),
        (RET, 0, 0, 0x0005_0000 | libc::ENOSYS as u32),
        (EQ, 0, 3, libc::SYS_clone as u32),
        (LOAD, 0, 0, 16),
        (SET, 0, 1, 0x7e02_0000),
        (RET, 0, 0, DENY),
        (LOAD, 0, 0, 0),
        (EQ, 0, 5, libc::SYS_ioctl as u32),
        (LOAD, 0, 0, 24),
        (EQ, 0, 1, 0x5412),
        (RET, 0, 0, DENY), // TIOCSTI
        (EQ, 0, 1, 0x541c),
        (RET, 0, 0, DENY), // TIOCLINUX
        (RET, 0, 0, 0x7fff_0000),
    ]);
    let mut bytes = Vec::with_capacity(code.len() * 8);
    for (op, yes, no, value) in code {
        bytes.extend(op.to_ne_bytes());
        bytes.extend([yes, no]);
        bytes.extend(value.to_ne_bytes());
    }
    Ok(bytes)
}

#[cfg(all(test, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod tests {
    use super::*;

    fn evaluate(filter: &[u8], arch: u32, syscall: u32, args: [u64; 6]) -> u32 {
        let mut data = [0u8; 64];
        data[..4].copy_from_slice(&syscall.to_ne_bytes());
        data[4..8].copy_from_slice(&arch.to_ne_bytes());
        for (i, arg) in args.iter().enumerate() {
            data[16 + i * 8..24 + i * 8].copy_from_slice(&arg.to_ne_bytes());
        }
        let mut accumulator = 0;
        let mut pc = 0;
        loop {
            let insn = &filter[pc * 8..pc * 8 + 8];
            let op = u16::from_ne_bytes(insn[..2].try_into().unwrap());
            let value = u32::from_ne_bytes(insn[4..].try_into().unwrap());
            pc += 1;
            match op {
                0x20 => {
                    accumulator = u32::from_ne_bytes(
                        data[value as usize..value as usize + 4].try_into().unwrap(),
                    )
                }
                0x15 | 0x45 => {
                    let yes = if op == 0x15 {
                        accumulator == value
                    } else {
                        accumulator & value != 0
                    };
                    pc += insn[if yes { 2 } else { 3 }] as usize;
                }
                0x06 => return value,
                _ => panic!("unexpected instruction"),
            }
        }
    }

    #[test]
    fn filter_enforces_abi_network_namespace_and_terminal_boundaries() {
        #[cfg(target_arch = "x86_64")]
        let arch = 0xc000_003e;
        #[cfg(target_arch = "aarch64")]
        let arch = 0xc000_00b7;
        let deny = 0x0005_0000 | libc::EPERM as u32;
        let allow = 0x7fff_0000;
        for network in [Network::Allow, Network::Deny] {
            let bytes = filter(network).unwrap();
            let call = |nr: libc::c_long, arg0: u64, arg1: u64, arg2: u64| {
                evaluate(&bytes, arch, nr as u32, [arg0, arg1, arg2, 0, 0, 0])
            };
            assert_eq!(
                evaluate(&bytes, 0, libc::SYS_read as u32, [0; 6]),
                0x8000_0000
            );
            #[cfg(target_arch = "x86_64")]
            assert_eq!(
                evaluate(&bytes, arch, 0x4000_0000 | libc::SYS_socket as u32, [0; 6]),
                0x8000_0000
            );
            assert_eq!(call(libc::SYS_read, 0, 0, 0), allow);
            assert_eq!(call(libc::SYS_clone, libc::SIGCHLD as u64, 0, 0), allow);
            assert_eq!(
                call(libc::SYS_clone, libc::CLONE_NEWUSER as u64, 0, 0),
                deny
            );
            assert_eq!(
                call(libc::SYS_clone3, 0, 0, 0),
                0x0005_0000 | libc::ENOSYS as u32
            );
            for nr in [
                libc::SYS_unshare,
                libc::SYS_ptrace,
                libc::SYS_mount,
                libc::SYS_open_tree,
                libc::SYS_keyctl,
                libc::SYS_io_uring_setup,
            ] {
                assert_eq!(call(nr, 0, 0, 0), deny);
            }
            for request in [0x5412, 0x541c] {
                assert_eq!(call(libc::SYS_ioctl, 0, request, 0), deny);
            }
            assert_eq!(call(libc::SYS_ioctl, 0, 0x5413, 0), allow); // TIOCGWINSZ
            for family in [
                libc::AF_INET,
                libc::AF_INET6,
                libc::AF_UNIX,
                libc::AF_PACKET,
            ] {
                assert_eq!(
                    call(libc::SYS_socket, family as u64, 1, 0),
                    if network == Network::Deny {
                        deny
                    } else {
                        allow
                    }
                );
            }
            assert_eq!(
                call(
                    libc::SYS_socket,
                    libc::AF_NETLINK as u64,
                    3,
                    libc::NETLINK_ROUTE as u64
                ),
                allow
            );
            if network == Network::Deny {
                assert_eq!(
                    call(
                        libc::SYS_socket,
                        libc::AF_NETLINK as u64,
                        3,
                        libc::NETLINK_AUDIT as u64
                    ),
                    deny
                );
                assert_eq!(call(libc::SYS_socketpair, libc::AF_UNIX as u64, 1, 0), deny);
            }
        }
    }
}
