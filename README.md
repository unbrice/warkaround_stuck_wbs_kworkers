# Stuck Writeback Workaround

A userspace daemon to mitigate indefinite `inode_switch_wbs` stalls in the Linux kernel.

This tool works around a kernel bug where writeback operations stall indefinitely, hogging gradually more and more CPUs, until there's none left. The daemon monitors `kworker` threads executing `inode_switch_wbs` that appear stuck and issues a `sync()` to free them up.

Why does this work? No idea. Ship it!

##  Are you affected?

This tool is for users experiencing a specific Linux kernel bug where writeback stalls indefinitely. If you are not experiencing this specific issue, this tool will not help.

Symptoms include:
- `watchdog: soft loookup... native_queued_spin_lock_slowpath` in `dmesg`
- `Workqueue: writeback wb_workfn` blocked on `native_queued_spin_lock_slowpath`, also in `dmesg`
- A growing number of `kworker/...+inode_switch_wbs` threads visible in `ps aux`

If your system exhibits these symptoms, this workaround may be effective. Otherwise, it is unlikely to help.

The bug is suspected to be related to XFS, but this is not confirmed.

<details>
<summary>Example Kernel Trace 1</summary>

```
Workqueue: writeback wb_workfn (flush-254:2)
Call Trace:
  ? asm_sysvec_apic_timer_interrupt+0x1a/0x20
  ? native_queued_spin_lock_slowpath+0x296/0x2d0
  _raw_spin_lock+0x3f/0x60
  __wb_update_bandwidth+0x35/0x1e0
  wb_update_bandwidth+0x52/0x80
  do_writepages+0x1c5/0x1f0
  ? native_queued_spin_lock_slowpath+0x220/0x2d0
  __writeback_single_inode+0x3d/0x370
  ? srso_alias_return_thunk+0x5/0xfbef5
  writeback_sb_inodes+0x1f5/0x4c0
  __writeback_inodes_wb+0x4c/0xf0
  ? srso_alias_return_thunk+0x5/0xfbef5
  wb_writeback+0x2d6/0x340
  ? srso_alias_return_thunk+0x5/0xfbef5
  wb_workfn+0x35b/0x520
  ? __schedule+0x404/0x1440
  ? srso_alias_return_thunk+0x5/0xfbef5
  ? srso_alias_return_thunk+0x5/0xfbef5
  process_one_work+0x18a/0x3a0
  worker_thread+0x28c/0x3b0
```

</details>

<details>
<summary>Example Kernel Trace 2</summary>

```
watchdog: BUG: soft lockup - CPU#13 stuck for 48s!
RIP: 0010:native_queued_spin_lock_slowpath+0x220/0x2d0
Call Trace:
 ? asm_sysvec_apic_timer_interrupt+0x1a/0x20
 ? native_queued_spin_lock_slowpath+0x220/0x2d0
 _raw_spin_lock+0x3f/0x60
 locked_inode_to_wb_and_lock_list+0x59/0x170
 __mark_inode_dirty+0x18d/0x390
 generic_update_time+0x4e/0x60
 xfs_vn_update_time+0xf7/0x1b0 [xfs]
 touch_atime+0xb0/0x120
 filemap_read+0x33f/0x370
 xfs_file_buffered_read+0x52/0xd0 [xfs]
 xfs_file_read_iter+0x71/0xe0 [xfs]
 vfs_read+0x210/0x360
 ksys_read+0x6f/0xf0
 do_syscall_64+0x39/0x90
 entry_SYSCALL_64_after_hwframe+0x78/0xe2
```

</details>


## Usage

Compile and run the daemon using Cargo. For verbose logging, include the `--verbose` flag.
```sh
# Ensure you have a Rust toolchain installed
cargo run --release -- --verbose
```

If Nix is available, just run `nix run`. For NixOS users, a flake is available. See [nix.md](nix.md) for details.

### Command-Line Arguments

- `--process-glob <GLOB>`: A glob pattern to identify the target `kworker` process names. (Default: `"kworker/*inode_switch_wbs"`)
- `--runtime-threshold <DURATION>`: The maximum permissible runtime for a monitored `kworker` process before triggering a `sync`. The value is parsed as a human-readable duration (e.g., `"30s"`, `"1m"`). (Default: `"30s"`)
- `-v`, `--verbose`: Enables INFO-level logging.
- `-d`, `--debug`: Enables DEBUG-level logging for maximum verbosity.
- `--no-timestamps`: Omit timestamps from log output.

### Polling Behavior

The daemon utilizes an adaptive polling strategy to minimize its own performance footprint:

- **Idle**: In the absence of any matching `kworker` processes, the daemon sleeps, awaiting process creation events from the kernel via a netlink socket.
- **Busy**: When a matching `kworker` is active but has not yet exceeded its time threshold, the daemon enters a tight polling loop, checking its status every second.
- **Recovery**: After triggering a `sync`, the daemon enters a 30-second cooldown period before resuming surveillance to allow the system to stabilize.


## License

This project is licensed under the GPL-3.0 or later. See the [LICENSE.txt](LICENSE.txt) file for details. 