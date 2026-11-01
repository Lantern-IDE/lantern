# lantern-ide

The command-line build of the [Lantern](https://github.com/Lantern-IDE/lantern) context engine. It indexes a code project on your machine and assembles the code an LLM needs for a question. No editor required.

```bash
npm install -g lantern-ide

lantern -C <project> index
lantern -C <project> context "how does login work"
lantern -C <project> mcp        # MCP server over stdio
lantern --help
```

To run it without installing, use `npx lantern-ide`. Do not use `npx lantern`: outside a project that has this package installed, it would fetch an unrelated package with that name.

Binaries come from platform packages such as `lantern-ide-win32-x64`, installed as optional dependencies. There is no postinstall script. Linux builds need glibc 2.34 or newer (Ubuntu 22.04, Debian 12, RHEL 9), so Alpine is not supported.

License: Apache-2.0.
