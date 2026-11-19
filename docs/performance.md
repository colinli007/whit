# Performance: idle memory after opening a project

English | [简体中文](performance.zh-CN.md)

Whit, Zed and VS Code each open the same project with their **out-of-the-box defaults** and are left alone; this compares their memory use.

## Results

| App | Memory (median) | Range | Processes | Relative to Whit |
|---|---|---|---|---|
| **Whit 0.1.0** | **79.4 MB** | 79.4–79.4 MB | 1 | 1× |
| **Zed 1.28.1** | **171.9 MB** | 171.9–172.0 MB | 2 | 2.2× |
| **VS Code 1.117.0** | **583.3 MB** | 568.3–604.5 MB | 9 | 7.3× |

### Per process (last sample)

**Whit**

| Process | Memory |
|---|---|
| whit | 79.4 MB |

**Zed**

| Process | Memory |
|---|---|
| zed (main process) | 169.2 MB |
| zed --crash-handler | 2.8 MB |

**VS Code**

| Process | Memory |
|---|---|
| Code Helper (Renderer): the window | 165.7 MB |
| Code: main process | 114.5 MB |
| Code Helper (GPU) | 108.4 MB |
| Code Helper (Plugin): extension host (built-in extensions only) | 73.9 MB |
| Code Helper: Node utility process | 50.5 MB |
| Code Helper: Node utility process | 29.7 MB |
| Code Helper (Renderer) | 22.7 MB |
| Code Helper (Plugin) | 20.5 MB |
| Code Helper: network service | 7.9 MB |

## Setup

- **Machine**: MacBook with Apple M4 and 16 GB of memory, macOS 15.8.
- **Project**: a React + Vite frontend project, 1813 tracked files, about 2.7 GB including `node_modules`; no uncommitted changes.
- **Defaults**: every app gets a brand-new empty profile, so no user settings, no third-party extensions, and no restored session (no terminals, AI agents and the like).
  - Whit: `HOME` points to an empty directory, so no recent projects or settings are read.
  - VS Code: `--user-data-dir` and `--extensions-dir` point to empty directories; only built-in extensions run.
  - Zed: `--user-data-dir` points to an empty directory.
- **Launch**: with an empty environment except `HOME`, `USER`, `SHELL`, `TMPDIR` and the system default `PATH`, as when opened from Finder; the project path is the only argument.
- **Idle state**: each app has only opened the project, with no file open, so no language server is running. Whit shows an empty Source Control view.

## Method

- **Metric**: `phys_footprint`, Activity Monitor's "Memory" column, read with `footprint -p <pid>`. It includes compressed and swapped-out memory, so under memory pressure it is closer to the real cost than RSS.
- **Scope**: the app's main process and all its descendants (the whole process tree), summed.
- **Sampling**: the apps are measured one at a time, never side by side. Each settles for 90 seconds after launch, then is sampled 12 times, 10 seconds apart; the median is reported. The process tree is then ended, and the next app starts 10 seconds later.

## Reproducing

```sh
scripts/memory_bench.sh <project path>             # Whit, Zed and VS Code
scripts/memory_bench.sh <project path> whit zed    # only some of them
```

It prints the results as Markdown to stdout and its progress to stderr. Apps are read from `/Applications` by default; override with `WHIT_APP`, `ZED_APP` and `VSCODE_APP`. `SETTLE`, `SAMPLES` and `INTERVAL` override the timing. Quit the apps being measured first: an instance already running may take over the project and make the new one exit, which the script reports instead of a number.

## Caveats

- During the test about 11.6 GB of swap was in use, so memory pressure was high. All three were measured under the same conditions and compare fairly, but absolute numbers may differ somewhat on a machine with free memory.
- This is the cost right after opening a project. Your own extensions, a restored session, and the language servers that start once a file is opened all raise VS Code and Zed considerably; Whit also grows when showing the diff of a large file.
- Whit is a git client for reviewing and committing changes, not a general-purpose editor. This compares the baseline cost of opening the same project, not equal functionality.

Measured on 2026-11-19.
