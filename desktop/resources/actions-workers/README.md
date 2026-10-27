This directory is populated before desktop bundling by:

    bash scripts/build-actions-workers.sh desktop/resources/actions-workers

The server transfers the matching pinned Actions worker through HostChannel.
Generated executables are ignored by Git. The desktop launcher passes this
bundled directory to its owned server; Go is not required on installed clients.
