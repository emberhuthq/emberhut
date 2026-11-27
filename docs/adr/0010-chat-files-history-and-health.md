# 0010. The chat's files, history pages, and the server's health

- Status: Proposed
- Date: 2026-11-26
- Amends: [ADR 0002](0002-rust-server.md): a message with files is taken (the files the server
  stored), and the server keeps a folder beside its database file.

## Context

The desktop app (through `emberhut-client`) and the mobile app make more calls than the server's
first slice served: they page back through a room's chat (`GET /api/history`), attach files
(`POST /api/uploads`, then the files with `POST /api/talk`), fetch them (`GET /uploads/…`, in
byte ranges for video), and a supervisor asks whether the server answers (`GET /health`). The
contract for all of these is in [api.md](../api.md); this ADR is about how the server keeps and
serves them.

The contract hands a client each uploaded file's absolute path, which it sends back with a
message. The health route reports how late the server's runtime runs, sampled once a second,
and `GET /api/loop` names the work that held it 50 ms or more.

## Decision

### Where the files are

The files live in `uploads/` beside the database file (the database's folder is the server's
data folder). There is no setting for it: the database's location is the one setting, and the
files move with it. The folder is made when the first file comes.

A file is noted in a `files` table (its stored name, its name as given, its kind, its size,
when it came) and a message reaches its files through `message_files` (message, position,
file), so a file can come with more than one message and the order is kept. No absolute path
is stored: a path is the uploads folder joined with the stored name, so moving the data folder
moves nothing in the database, and an imported archive's attachments need no rewriting.

This server's `files` is the chat's files. A later table that tracks how far an index has read
each session transcript takes another name (`transcript_offsets`, say).

### Storing

The body of `POST /api/uploads` is the raw file, written to disk as it arrives and never held in
memory. A `Content-Length` over 500 MiB is refused before anything is read; a body without one is
counted as it arrives and refused once it passes the limit. A body that fails, or that stops
arriving for 60 s, is given up. In every refusal the part written is deleted: a part of a file is
never kept. The server-wide request timeout does not apply to an upload, since a phone's video takes
minutes; the 60 s idle limit takes its place.

The stored name is 8 random hex digits, `-`, and the name made safe (`emberhut_core::upload`),
so names do not collide and never leave the folder. The answer gives the file's absolute
`path`, which the client sends back whole.

### Taking files with a message

A file sent with `POST /api/talk` is taken when its `path` is directly in the uploads folder, the
file is there (a regular file, not a link), and the server noted it in `files`; the others are
dropped without an error. The last check closes a gap: a path the client makes up for a file someone
put in the folder by hand is not attached. A message is blank, and refused, only when its text is
blank and no file was taken; files alone are a message. The assistant's turn gets the files with
their paths (`Turn::files`), and `services::uploads::describe` words them under the person's
message.

### Serving

`GET /uploads/<stored name>` is behind the same sign-in as the rest (`?token=` for players that
can't send a header). Any regular file in the folder is served, noted or not, so files an imported
archive brings in are served too; a name with `..` or a separator, a link, or a missing file is 404
with no body. One byte range is read (`bytes=a-b`, `a-`, `-n`; 206, or 416 past the end); several
ranges or another unit get the whole file. An HTML file is served with a sandboxing
`Content-Security-Policy`, `nosniff` and `no-referrer`. A whole file may be cached for an hour (a
stored file never changes).

### History

`GET /api/history` reads `before` and `after` leniently (a bound
that isn't a number is no bound, a fraction bounds at the next whole id), so no query is a
400. A page is read in one transaction with the messages its replies quote and every file they
show, and lines are built by the same code as the state's `chat`.

### Health

`GET /health` answers `{ ok: true, lag }` without sign-in: it says nothing about anyone's data, and
a supervisor has no session. `GET /api/loop` is behind sign-in. The lag is measured on the server's
async runtime: a probe asks the runtime to run a step once a second and notes how long the step
waited for a worker. Slow work is a single poll of a request's handler that held a worker 50 ms or
more, noted under the request's method and path with ids as `:x`. Once a minute a lagging minute is
logged, and three minutes in a row with a 95th percentile of a second or more are logged as a stall.
The page keeps 300 samples and 200 slow works.

## Consequences

- The data folder now holds a folder as well as the database file: a backup copies both.
- Deleting a message deletes its links to files (cascade) but not the files; a file still
  linked to a message cannot be deleted. When a delete route comes it decides whether unlinked
  files are removed.
- A file put in the uploads folder by hand is served but cannot be attached; importing an archive
  notes the files it copies in.
- `/health` is public: a server reachable from the internet tells anyone that it runs and how
  late its runtime is, and nothing else.
