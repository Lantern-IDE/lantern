# Security Policy

Lantern runs AI agents that can read files, edit code, and run commands on your machine, so security reports are taken seriously.

## Reporting a vulnerability

Please report privately through **GitHub → Security → Report a vulnerability** on this repository. Do not open a public issue.

Include what you found, how to reproduce it, and the impact you expect. You should get a reply within 7 days. This is a one-person project, so fixes may take longer; you will be kept informed.

## Scope

Especially relevant:

- Ways for a project, file, or model response to make Lantern write outside the project folder, write to `.git`, or run commands without approval
- Restricted mode (untrusted folders) not blocking project settings, hooks, or write/exec tools
- API keys or secrets leaking into logs, problem reports, or model requests despite masking
- Update signature verification bypasses

## Supported versions

Only the latest release receives fixes during the beta.
