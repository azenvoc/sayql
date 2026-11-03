# Security policy

SayQL connects to databases and stores passwords and API keys, so security reports are taken seriously.

## Reporting a vulnerability

**Please don't open a public issue.** Report it privately through GitHub:
[Security → Report a vulnerability](https://github.com/azenvoc/sayql/security/advisories/new).

Include what you found, how to reproduce it, and what an attacker could do with it. You'll get a reply
within a few days. Once a fix is released, you'll be credited in the advisory unless you'd rather not be.

## In scope

Anything that could let SayQL, or its MCP server, change data without the person's confirmation; leak
passwords, API keys or table rows; run SQL the person didn't approve; or install an update that isn't
signed with SayQL's update key.

## Supported versions

Fixes go into the latest release. Installed copies update themselves from the status bar.
