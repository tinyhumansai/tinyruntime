# TinyRuntime process boundary

This private crate binds a Unix child command to an opened directory. It clones
and owns the descriptor when configuring the command, then calls `fchdir` in the
child before exec. Renaming or replacing the original pathname cannot redirect
the child, and the parent process working directory never changes.

Darwin's `/dev/fd` does not provide the directory traversal semantics of Linux's
`/proc/self/fd`. Using the syscall avoids either filesystem namespace.

The crate isolates the required unsafe `pre_exec` registration. Unsafe code is
denied except on that one documented function; TinyRuntime and the other
workspace members continue to forbid unsafe code. The registered closure only
performs the async-signal-safe syscall and errno conversion. Descriptor cloning
happens before fork, and ownership keeps the descriptor alive until spawning
finishes. No allocation, locks, environment access, or path reopening occurs in
the child callback.

Tests spawn real local processes with a renamed directory and a symlink
replacement, reuse the command after the original descriptor is dropped, and
verify rejection of a regular file as a working directory.
