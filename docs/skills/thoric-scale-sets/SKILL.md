---
name: thoric-scale-sets
description: How to work through a Thoric scale set, an MCP endpoint such as /mcp/<set> whose tools take an `instance` number and that offers list_instances, create_instance and delete_instance. Use it whenever those tools are in the tool list, whenever work could be split across several desktops or parallel subagents ("give each agent its own desktop", "run these in parallel", "spin up another environment"), whenever a subagent is handed an instance number, and whenever several instances or agents may write the same shared folder, even when the user never says "scale set".
---

# Thoric scale sets

A scale set is one desktop container made scalable. Its endpoint, `/mcp/<set>`, drives every instance of it: each tool takes `instance`, the instance's number, and `create_instance` adds a new numbered desktop on demand. A new instance is not a copy. It is created from the same image, settings and compose values as instance 1, runs the compose setup itself and starts with none of the other instances' files. What instances have in common are the volumes the set shares.

That gives two things to get right, and they pull against each other:

1. **Growing for concurrent work.** The point of a set is that several agents work at the same time, each on a desktop of its own, without stepping on each other's screen, input or home folder.
2. **Sharing volumes carefully.** A shared folder is the same volume in every instance. Nothing in Thoric coordinates writes to it: two instances writing one file is a race, and a delete there deletes it for everyone. Concurrency is only safe when the work is laid out so that it does not collide.

## Start with list_instances

Call `list_instances` before anything else, and again whenever you are unsure what exists. It answers:

- `instances`: each one's `instance` number, `name` (such as `re-3`), `note` (what it is for and who uses it), `state` (`running`, `exited`), `creating` (its job has not finished), `kept` (one of the lowest numbers the set always keeps), `in_use`, and its `mounts`, each with `sharing`: `shared` with another instance or `own`.
- `slots`: the set's volumes by the folder they are mounted at (`target`) with the `policy` a new instance gets: `shared` (the same volume everywhere), `new` (an empty one of its own, deleted with it), `first` (instance 1 keeps the original, the others get their own) or `first_writes` (the same volume everywhere, which only instance 1 can write; the others mount it read-only).
- `free`: how many more instances the set may have before it reaches its maximum.
- `servers`: the desktop servers the set runs in instance 1 alone for every instance (`first`); see [A server that runs in instance 1 alone](#a-server-that-runs-in-instance-1-alone).

The endpoint's own instructions also name the shared folders and the per-instance ones. Read both before you plan, because the plan depends on which folders are shared.

Check the tool list too. Your client may put the MCP server's name in front of each tool, such as `re_create_instance` in OpenCode or `mcp__re__create_instance` in Claude Code; they are the same tools. `create_instance` and `delete_instance` exist only when the endpoint has the `scale` scope, and `exec_container` and the terminal tools only with `commands`, which a set's endpoint does not get by default. An endpoint can also carry a tool profile: a headless one offers no screenshot, window or input tools, and a desktop one no terminals or diagnostics. Work with what is offered instead of assuming.

## Decide whether to grow

Use one instance per worker. Two agents on one desktop share its screen, mouse, keyboard, clipboard, windows and home folder, so their clicks and keystrokes interleave and each reads the other's state as its own. That is the collision a set exists to avoid.

- **One agent, sequential work:** use an existing instance. Pick one whose note says it is yours or free; leave alone one whose note names another agent or task. Instance 1 is often the user's own desktop, so do not take it over unless the user asked.
- **Parallel work:** create one instance per concurrent worker. Size it to the parallelism the work can really use, not to the maximum: every instance is a full desktop with its own memory and CPU on a host other people use, and each runs the compose setup, which can take minutes. If `free` is smaller than what you planned, plan for fewer workers or ask the user to raise the set's maximum (at most 16) on the dashboard.

## Grow: create the instances

For example, with a set whose browser profile folder is one of its slots:

```
create_instance(note: "worker 2: decompile network module", volumes: {"/home/desktop/profile": "new"})
-> {"instance": 3, "name": "re-3", "job_id": "...", ...}
```

- **Write a note every time.** One line, at most 200 characters, saying which agent or task the instance is for. It is how you, other agents and the user tell instances apart in `list_instances` and on the dashboard, and how you avoid deleting someone else's.
- **The number arrives at once, the desktop later.** Wait for `get_job` with `wait_seconds` (up to 300; call again if it is still running) to report `completed` before you or a subagent use the instance. Until then its tools fail or find a desktop whose setup has not run.
- **Start creations together, then wait.** Calls to `create_instance` can follow one another without waiting: each takes its own number under the set's lock and builds in its own job, in parallel. Issue all of them, do your planning while they build, then wait on each job.
- **Choose volumes for this instance.** `volumes` maps a slot's `target` to `shared` or `new` for this instance alone. Ask for `new` for a folder this worker must have to itself (see below), and keep `shared` where it should see or publish common files. Only folders listed in `slots` can be named, and `first` and `first_writes` are set policies, not choices per instance: asking `shared` for a `first_writes` folder still mounts it read-only after instance 1.
- **A failed creation:** read the job's log with `get_job`, delete the instance if `list_instances` still shows it, and try again once. If it fails the same way, stop and tell the user what the log says instead of looping.

## Hand an instance to a subagent

Give each subagent everything it needs to stay in its lane. The names, number and folders below are examples; take the real ones from `list_instances` and the endpoint's instructions:

```
You work on instance 3 of the Thoric scale set "re" (MCP server thoric-re).
Pass "instance": 3 to every tool call; desktop server tools named SERVER__TOOL take it too
(as "thoric_instance" when the tool has an "instance" argument of its own).
Do not call create_instance or delete_instance, and never pass another number.
Your own folders: /home/desktop (private, deleted with the instance).
Shared with other workers: /data. Write only under /data/work/instance-3/;
read the rest, never change or delete it.
When you finish, write /data/work/instance-3/DONE and report what you produced.
```

The subagent should not create or delete instances itself; the parent that grew the set shrinks it, so that nothing is deleted while another worker still needs it.

## Share volumes without collisions

A shared folder behaves like one disk that several machines write at once: writes are visible to everyone immediately, the last writer wins, and nothing locks a file for you. Lay out the work so that no two workers write the same path.

**Give every worker its own subfolder.** Each instance writes under a path that carries its number or its task, such as `/data/work/instance-3/` or `/data/results/<task-id>/`, and treats the rest of the shared folder as read-only. This alone removes most races.

**Keep one writer for common files.** An index, a summary, a shared notes file or a results table has exactly one writer, usually the parent. Workers write their own files and the parent merges them. Never have several workers append to one file: their lines interleave or overwrite each other.

**Publish a file whole.** `write_file` writes atomically, so a reader never sees half of it, and without `replace` it refuses a file that already exists. Something written in several steps, such as a program's output, goes into a temporary name first and is moved into place with `move_file` in the same folder. Write a marker such as `DONE` last, and have readers wait for the marker instead of guessing whether the files are complete.

**Claim shared work with a file.** When workers pull tasks from a common list instead of being handed them, `write_file` of `/data/claims/<task>.json` without `replace` is a lock: exactly one instance creates it, and every other gets a refusal and moves on to the next task. Put the instance number and a time in it, so a claim left by a worker that died can be recognized.

**Give a folder to one instance when a program locks its data.** Many programs assume they are the only one using their folder and break, refuse to start or corrupt data when a second copy opens it from another instance:

- browser profiles (Chrome and Chromium refuse or damage a profile another instance has open);
- a git working tree (two instances running git in one checkout fight over its index and lock files; give each worker its own clone or `git worktree` and its own branch);
- SQLite and other database files (keep one writer, or one instance owning the database);
- editor and engine project state such as a Unity project's `Library`, a Wine prefix, a build's output folder.

For such a folder, create the instance with `volumes: {"<target>": "new"}`, or point the program at a path inside the worker's own subfolder. When the program is a desktop server every worker needs on the same data, such as Ghidra on one shared project, a copy per instance is the wrong answer: ask the user to set that server to "Instance 1 only" in the set's settings, which makes it one program for all instances (next section).

## A server that runs in instance 1 alone

A set can run a desktop server in instance 1 alone, for every instance: the set's `servers` name it as `first`, and the endpoint's instructions list it. Thoric then never starts it in another instance, and every agent and endpoint shares the one program in instance 1. Its tools on the set's endpoint:

- **take no `instance`.** Whatever instance you work in, a call goes to instance 1. Do not try to open the same project in your own instance with another program; that is the collision this setting prevents.
- **see instance 1's desktop and files.** A path you pass must exist in instance 1: put inputs in a shared folder (your own subfolder there) and pass that path, never a path in your instance's home or in a `new` volume.
- **share one state.** The program keeps what the last caller left, such as the binary Ghidra has open or the program it analyzed. Before working, check or set the state you need (open your file explicitly), and do not assume another agent is not using it at the same time. When several workers need it, have them take turns or let one worker (often the parent) drive it and hand out results.
- **depend on instance 1.** If instance 1 is stopped, a call says so; start it with `container_action` and `instance: 1` where the endpoint offers the `power` scope, or ask the user. Never delete instance 1 to make room.

Its tools use instance 1's desktop, so a window it opens appears there, not in your instance: take screenshots of instance 1 when you need to see it.

**A folder only instance 1 writes.** Under `first_writes` every instance sees the folder, but only instance 1 can change it; the others get "Read-only file system" for any write, also as root. Use it as a reference library: have the parent publish inputs there from instance 1, and have workers read them and write their results to their own folders or to another shared one. Do not try to work around it from another instance, and do not ask for `new` to get write access unless the worker really needs an empty folder of its own there.

**Deleting in a shared folder deletes for everyone.** Never remove, move or overwrite shared files you did not create, and clean up only your own subfolder. `delete_instance` never touches shared volumes, so cleaning them is a deliberate step, not a side effect.

**Private means gone with the instance.** The home folder and every `new` or `own` volume belong to that instance alone and are deleted with it. Anything that must outlive a worker, results, logs, recordings, goes into a shared folder (or to the user through `download_file`) before the instance is deleted.

## Shrink when the work is done

1. Make sure each worker's results are in a shared folder or handed to the user.
2. `delete_instance(instance: N)` for every instance you created, then `get_job` until it completes. It deletes the instance with the volumes made for it; shared volumes stay.
3. Leave alone instances you did not create (check their notes) and the `kept` ones, which an agent cannot delete and the set refuses anyway.

**Numbers are reused.** A new instance takes the lowest free number, so after `delete_instance(3)` the next `create_instance` may be instance 3 again, for another agent. Drop a number once its instance is deleted, never hand a stale one to a subagent, and when you come back to a number after a long time, check with `list_instances` that its note is still yours.

**Idle instances disappear.** A set may delete an instance beyond the kept ones once nothing used it for its idle time, together with its own volumes, and an instance may also be stopped when idle. Tool calls that name it, its jobs, an open viewer and a busy CPU count as use; an agent thinking for a long time does not. So save results to a shared folder as you go, not only at the end. A stopped instance (`state` `exited`) starts again with `container_action` where the `power` scope is offered; a deleted one has to be created again.

## When something is refused

| Answer | Meaning and what to do |
| --- | --- |
| `create_instance`: the set has its most instances | Wait for a worker to finish and delete its instance, reuse one that is free, or ask the user to raise the maximum. Do not delete another agent's instance to make room. |
| `delete_instance`: still being created | Wait for the creation's job, then delete. |
| `delete_instance`: one of the instances the set keeps | Leave it; only the dashboard deletes it. |
| A tool says the set has no such instance | It was deleted, perhaps for being idle. Run `list_instances` and create a new one if the work needs it. |
| A tool is unknown | The tool list changed (a desktop server was added or removed); list the tools again. |
| A server tool says the set runs it in instance 1 alone and instance 1 does not run | Start instance 1 with `container_action` where `power` is offered, or ask the user; never create or delete instances to get it back. |
| A creation job failed | Read its log, delete the instance, retry once, then report. |

## On the administrator's /mcp

The administrator's `/mcp` has the same `list_instances`, `create_instance` and `delete_instance`, taking the set's name as `set`, and its container tools take the instance's `container_id` (from `list_instances`) instead of `instance`. A set is made by `create_container` with `scaling`: `set` (its name), `min` (the instances it keeps, 1 by default), `max` (4 by default, at most 16), `idle_delete_minutes` (0, never, by default), `volumes`, the policy of each folder (`shared` by default, `new`, `first` or `first_writes`), and `servers`, where each desktop server runs (`each` by default, or `first` for instance 1 alone). Choose the policies with the same care as above: `shared` for what workers must see of each other, `new` for any folder a program locks, `first_writes` for material workers read and must not change, and `first` for a desktop server that every worker uses on one shared project. `call_mcp_server_tool` on another instance than 1 is refused for such a server; call it with instance 1's `container_id`.
