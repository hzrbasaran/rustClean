# Security policy

rustClean moves files to the trash and can run cleanup commands of developer
tools, so we treat these as security issues:

- anything that deletes, moves or offers to delete data the user did not
  choose (for example a report listing an installed app's data as a leftover);
- running a command or deleting without the confirmation step;
- a way to make rustClean run unintended commands or touch paths outside what
  it shows.

## Supported versions

Only the latest release receives fixes.

## Reporting a vulnerability

Please **do not open a public issue**. Instead, use one of:

- GitHub's private reporting: **Security → Report a vulnerability** on the
  [repository page](https://github.com/hzrbasaran/rustClean/security/advisories/new);
- e-mail: hzrbasaran@hotmail.com.

Include the version or commit, your operating system, and steps to reproduce.
You can expect a first answer within a week. Once a fix is released, we will
credit you unless you prefer otherwise.

If rustClean listed something it should not have, please also say which report
it was and the path involved (you may anonymize user names). The data itself is
not needed.
