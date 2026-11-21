# How Purgeon measures things

This page says where every number comes from, how it is calculated, and where it can be wrong.
Measured results from the bench are at the end, under [Measured results](#measured-results).

Purgeon reads all of its data from system-wide sources: `NtQuerySystemInformation`, Windows
performance counters (PDH), DXGI, NVML on NVIDIA, and, when running as administrator, a kernel
trace session for disk I/O. On another process it only ever opens a handle with
`PROCESS_QUERY_LIMITED_INFORMATION` (the same right Task Manager uses) to read the path of its
executable. It never reads or writes another process's memory, and never injects, attaches or
debugs. The one exception is **End task**: only after you confirm, Purgeon asks for
`PROCESS_TERMINATE` on that one process and calls `TerminateProcess`.

## Units and rounding

- Bytes use binary units, the way Task Manager does: 1 KB = 1,024 bytes, 1 MB = 1,024 KB.
  Under 1 MB values have no decimals, MB has one decimal, GB has two.
- Rates (`/s`) are the bytes moved during the last update divided by the real time the update
  took, not the nominal interval.
- Percentages have one decimal. Values below 0.05% show as `0` so idle rows stay quiet.
- "—" means the value could not be read. It never means zero.

## CPU

### Total CPU ("Utilization")

`GetSystemTimes` gives idle, kernel and user time since boot. Between two updates:

```
busy % = 100 × (Δkernel + Δuser − Δidle) / (Δkernel + Δuser)
```

This is the same quantity as perfmon's `\Processor Information(_Total)\% Processor Time`.
Per-core values use `SystemProcessorPerformanceInformation` with the same formula.

### "Task Manager figure"

Task Manager's CPU number is `% Processor Utility`, not `% Processor Time`. Utility multiplies
busy time by how fast the clock ran compared with the base clock. A CPU boosting to twice its
base clock at 30% busy shows about 60% in Task Manager. Purgeon reads that counter directly and
shows it on the CPU page so the two can be compared, with a tooltip that explains the difference.
Neither number is wrong. They answer different questions: "how much of the time was the CPU
busy" versus "how much work was done compared with running flat out at base clock".

### Per-process CPU (the cycles formula)

Each `SYSTEM_PROCESS_INFORMATION` record carries `CycleTime`, the processor cycles charged to
that process's threads. `SystemProcessorIdleCycleTimeInformation` (class 83) gives the cycles
each logical processor spent in its idle thread. Between two updates:

```
cpu %(process) = 100 × ΔCycleTime(process) / ( Σ ΔCycleTime(all processes except Idle) + Σ Δidle cycles(all logical processors) )
```

The denominator is every cycle every logical processor ran during the interval, busy or idle,
so the per-process values add up to the total busy share. This is the method Process Explorer
uses.

**Why cycles instead of CPU time.** The `KernelTime` and `UserTime` fields that Task Manager's
Processes tab is based on are charged in whole clock-interrupt ticks (usually 15.6 ms). A thread
that runs for 2 ms and sleeps before the tick may be billed nothing, or the whole tick may land
on whichever thread happened to be running. Cycle counts are taken at every context switch, so
short bursts are counted and charged to the right process.

**Boosted cores.** On current Intel and AMD CPUs Windows reads the invariant time-stamp counter
for `CycleTime`. That counter ticks at a fixed rate whatever the real core clock is, so a cycle is
effectively a unit of time. Turbo boost therefore does not inflate a process's share: a process
that keeps one of 28 logical processors busy shows about 3.6%, at base clock or at full boost.

**Parked and sleeping cores.** A parked core runs its idle thread, and its idle cycles keep
counting while it sleeps. It counts as idle capacity in the denominator, the same way `% Processor
Time` counts it.

**Fallback.** If class 83 is unavailable, or a process appeared or disappeared during the
update, Purgeon falls back to the timer-based formula for that update:
`Δ(kernel + user) / (interval × logical processors)`.

**Pending verification:**

- Hybrid CPUs (P-cores and E-cores): the TSC is shared, so the formula should hold, but this has
  been checked on only one machine (i7-14700F). On that run the sum of per-process values was
  13.8% against perfmon's `% Processor Time` of 13.8%.
- Older CPUs without an invariant TSC: here cycles may scale with frequency, and per-process
  values would lean towards processes that ran on boosted cores. Not tested.

### Clock speed

`\Processor Information(*)\% Processor Performance` × base clock (from the registry
`~MHz` value) gives the effective clock of each logical processor. This is how Task Manager
computes its "Speed".

## Memory

### Per process

The Processes page has one memory column, **RAM**: how much physical memory the app holds right
now. Pointing at a number shows what it is made of.

| Shown as | Source | Meaning |
|---|---|---|
| **RAM** | `WorkingSetSize` | Every page of RAM the process holds: its own, plus pages it shares (DLLs, mapped files, shared memory). This is what lowers available RAM. |
| Only this app | `WorkingSetPrivateSize` | RAM only this process uses. Task Manager's "Memory" column. |
| Shared with other apps | `WorkingSetSize − WorkingSetPrivateSize` | Pages other processes may map too. |
| Reserved | `PrivatePageCount` (commit) | Private memory the process has asked Windows to set aside, in RAM or the page file. It can be much larger than RAM use, and grows when a program leaks memory. |

**Why RAM and not Task Manager's number.** Task Manager's "Memory" leaves out shared pages, so an
app that loads big files, or uses shared memory (browsers, game engines and graphics drivers
do), can lower available RAM by far more than Task Manager shows. Commit is no better as a
headline: memory that is reserved but never written uses no RAM at all. The working set is the
one figure that tracks what available RAM loses, which the test below shows.

**Groups.** With "Group by name" on, a group's RAM is every member's private memory plus the
*largest* member's shared memory. Processes of one program mostly share the same pages
(the same DLLs, the same shared memory), so adding up their shared parts would count those
pages many times; 30 browser processes could otherwise appear to hold gigabytes they don't.
This is an estimate: a group whose members share different pages reads a little low. The same
rule is used for "Top memory" on the Dashboard.

**Tested** (`cargo run --release --features bench -- --bench-mem`, two runs, same machine as
below). A small helper process reserved or used memory of a known size; the table shows how
much each figure rose, and how much available RAM fell:

| What the helper did | Only this app (Task Manager) | RAM (Purgeon) | Reserved | Available RAM fell by |
|---|---|---|---|---|
| Reserved 512 MB, never used it | 0 MB | 0.1 MB | 513 MB | 12 MB, 47 MB (noise from other apps) |
| Used 256 MB of shared memory | 0 MB | 256 MB | 0.5 MB | 257 MB, 254 MB |

Task Manager's number missed the shared memory completely, and so did commit, while available
RAM fell by the full 256 MB. Purgeon's RAM figure matched it. The reserved-only memory moved
commit but not RAM, as expected. The private working set also matched perfmon's
`Working Set - Private` exactly (see [Measured results](#measured-results)).

### System

`GlobalMemoryStatusEx` and `GetPerformanceInfo` give total, available, commit, cache and kernel
pools. As administrator, `SystemMemoryListInformation` (class 80) adds the free, zeroed,
modified and standby lists. "In use" is total minus available, like Task Manager.

## GPU and VRAM

All GPU data comes from the counters Windows' graphics kernel (dxgkrnl) publishes, the same data
Task Manager's GPU columns use:

- **GPU %, per process:** `\GPU Engine(*)\Utilization Percentage`, summed per process per
  engine. A process's figure is its busiest engine, the way Task Manager reports it.
- **Dedicated VRAM, per process:** `\GPU Process Memory(*)\Dedicated Usage`, summed over all
  adapters.
- **Shared GPU memory, per process:** `\GPU Process Memory(*)\Shared Usage`. This is system RAM
  the GPU can use. It is kept separate from dedicated VRAM and never added to it.
- **Per adapter:** `\GPU Adapter Memory(*)\Dedicated Usage` and `Shared Usage`, with totals from
  DXGI (`DXGI_ADAPTER_DESC1`).
- **Temperature, clocks, fan and power:** NVML on NVIDIA cards only. Other vendors show "—".

If the GPU counters cannot be opened (some virtual machines, remote sessions, or very old
drivers) the GPU and VRAM columns show "—", not 0.

Known limits: the counters are refreshed by the driver about once a second, so with a 0.5 s
update speed every second reading repeats. Memory a driver allocates on a process's behalf may be
charged to a different process, for example `dwm.exe`.

## Disk

- **Per drive:** `\PhysicalDisk(*)` counters: read and write bytes per second, active time, average
  response time and queue length. Task Manager and Resource Monitor show the same counters. Purgeon
  puts them all in one view.
- **Per process, standard user:** the "I/O" column is `ReadTransferCount + WriteTransferCount` per
  second. That counts all I/O a process issues: files (including reads served from the cache),
  network and devices. It is not disk activity.
- **Per process, administrator:** a private kernel trace session ("Purgeon Disk Trace") records
  DiskIo read and write events: only I/O that reached the physical disk. Each event is charged to
  the process of the thread that issued it. I/O from a process that exited before the update is
  listed separately. Writes the system flushes later from the cache are charged to `System`, the
  same as in Resource Monitor.

## How to check these numbers yourself

```
cargo run --release --features bench -- --bench
```

This writes `bench-results.md`. It has two parts and takes about two minutes:

1. **Idle:** 20 seconds of Purgeon's figures next to perfmon's for the same moments, and
   Purgeon's own cost.
2. **Known loads:** a copy of Purgeon (`purgeon-load.exe`, in the temp folder, below-normal
   priority) does a measured amount of work and reports it. The bench then checks how much of it
   Purgeon saw, and how much the Windows counter Task Manager is built on saw. The loads are kept
   small: 4 threads of 28 spinning for 15 s, the same 4 threads running 3 ms on and 10 ms off,
   512 MB of RAM held for 8 s, and 256 MB written to a delete-on-close temp file at 32 MB/s. Each
   load exits on its own within 60 s. Nothing touches the GPU, needs administrator or loads a
   driver.

## Measured results

i7-14700F (28 logical processors), Windows 11, standard user, two runs on 21 November 2026.
"Error" is (seen − truth) / truth. For CPU, the truth is the time the load's threads measured
themselves running, with any pause over 50 µs left out.

| Load | Truth | Purgeon (run 1, run 2) | Windows counter (run 1, run 2) |
|---|---|---|---|
| Steady CPU, 4 threads for 15 s | 59.8 CPU-seconds | +0.2%, +0.9% | −0.7%, −0.3% (`Process % Processor Time`) |
| Bursty CPU, 3 ms on, 10 ms off | 13.5 CPU-seconds | +0.3%, +1.7% | −1.1%, −1.5% |
| 512 MB allocated and touched | 512.0 MB | 512.0 MB, exact | 512.0 MB, exact (`Working Set - Private`) |
| 256 MB written by one program | 256.0 MB | +0.1%, −0.1% | 0.0%, 0.0% (`IO Write Bytes/sec`) |

Idle, 20 samples at 1 s:

| Check | Mean | p95 |
|---|---|---|
| Total CPU vs `% Processor Time` (points) | 0.52 | 1.08 |
| Sum of per-process CPU vs total (points) | 1.40 | 3.70 |
| Task Manager's `% Processor Utility` minus `% Processor Time` (points) | 0.76 | 3.12 |
| Private working set vs perfmon, processes over 50 MB (error %) | 0.002 | 0.000 |
| Time per update (ms) | 12.6 | 16.8 |
| Purgeon's own CPU, sampler only (%) | 0.066 | 0.073 |

Purgeon with its window open used about 51 MB of private working set during these runs. That
was a bug: the clean-up that hands back startup memory was skipped when the window opened
without focus. With it fixed, Purgeon holds about 33 MB of RAM open (23 MB private) and about
27 MB minimized, measured after two minutes. Most of what remains reserved but out of RAM is
NVIDIA's sensor library, which sets aside about 19 MB when it starts; it is only needed for
GPU temperature, clocks, power and fan.

**What this shows.** On this machine Purgeon is as accurate as Windows' own counters, not more.
Both stay within 2% of a known CPU load. Purgeon reads slightly high and the Windows counter
slightly low, by about the same amount. The bursty test was meant to catch the 15.6 ms timer-tick
undercount described above. It did not show up here: the Windows counter was within 1.5%.
Something on the PC may have raised the timer rate, which makes tick accounting finer, so other
machines may still differ. Private memory and per-program disk writes match exactly, because Purgeon and
Task Manager read the same values. (Purgeon's RAM column also counts shared memory; see Memory.)

Not tested yet: GPU and VRAM, administrator mode (per-program physical disk and network), and
other CPUs.
