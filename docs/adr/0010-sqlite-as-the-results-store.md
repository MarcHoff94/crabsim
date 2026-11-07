# Results are kept in an SQLite database in the models folder

Every simulation run started through the operation catalogue writes its results to one SQLite database, `<models folder>/.crabsim/results.sqlite`: the run's settings, the canonical text of the model it ran and its hash, each observation, and each figure of each object. People and agents read it through the same operations, `run_results` for a summary and `query_results` for read-only SQL, and later the UI's results views read it too. We chose a database because results have to survive restarts, because an agent should be able to ask any question of them in one call rather than paging through a 6,000-object model's figures, because SQL is a query language every LLM writes well, and because the CLI, the MCP server and later the UI should all see the same runs. The store is compiled into crabsim with `rusqlite`'s `bundled` feature, so nothing has to be installed.

## Considered Options

- **Results only in memory,** held by the process that ran them. Nothing to design or keep stable, but results vanish when the MCP server or CLI exits, a person can't see an agent's runs, and every question an agent asks needs an operation of its own.
- **A JSON file per run** in the models folder. Readable without any tool and easy to write, but comparing runs or models means reading and joining many files, which an agent would do by hand, a few figures at a time, and which the UI would have to rebuild as an index anyway.

## Consequences

- **The cost:** `rusqlite` with SQLite built in adds a C build of about 30 s to a clean build of the default members, and a few hundred kilobytes to the binaries. A run writes its results once per observation, at its end, so the per-event path and the benchmark are unchanged. A headless run without a models folder, such as `crabsim --length 365d`, writes nothing.
- **The tables are an interface.** Skills, scripts, people's own SQL and the UI rely on the tables and statistic names, so they stay stable: `statistics` is long and thin, one row per figure, so a new statistic needs no new column. A change to the tables raises the version kept in SQLite's `user_version`, and a crabsim that finds a newer store refuses to write to it rather than damage it.
- **WAL mode,** so a CLI and an MCP server can use the same store at once: readers never wait for a writer.
- **Queries can't change the store or hang a front-end:** `query_results` runs one `SELECT` on a read-only connection with no attached databases, capped at 1,000 rows and 5 seconds.
- **The store holds what a run needs to be reproduced:** the canonical model text and its SHA-256, the seed, length, warm-up, number of observations and crabsim's version. The Init state and everything after it follow from those, so they aren't stored.
- **Results stay readable in any SQLite tool,** such as DB Browser for SQLite or the `sqlite3` shell, before crabsim's own results views exist.
