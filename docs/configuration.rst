Configuration
=============

cortex-dispatcher
-----------------

Cortex Dispatcher is configured using a `YAML <http://yaml.org/>`_ based
configuration file. Here is an example configuration:

.. code-block:: yaml

    storage:
      directory: /test-data/storage

    command_queue:
      address: "127.0.0.1:5672"
      queue_name: cortex-dispatcher

    directory_sources:
      - name: mixed-directory
        directory: /test-data/source
        targets:
          - regex: "^.*-red\\.xml$"
            directory: /test-data/red
          - regex: "^.*-blue\\.xml$"
            directory: /test-data/blue

    tar_extractors:
      - directory: /test-data/source
    tar_extractors_enabled: true

    sftp_sources:
      - name: local-test
        address: 127.0.0.1:22
        username: cortex
        thread_count: 4

    prometheus:
      push_gateway: 127.0.0.1:9091
      push_interval: 5000

    sqlite:
      path: "/var/lib/cortex/cortex.db"

Each configured ``tar_extractors`` directory is scanned recursively at
``scan_interval`` for ``.tar.gz`` archives. Archives are extracted into their
containing directory and removed after successful extraction, before extracted
files are picked up and hashed by directory sources. Configure directory source
filters to match extracted files rather than the ``.tar.gz`` archives. For
``.gz`` members, ``unpack_before_hash`` controls whether their decompressed
contents are hashed. Set ``tar_extractors_enabled`` to ``false`` to disable
extraction while leaving the directory configuration in place.


cortex-sftp-scanner
-------------------

Cortex SFTP Scanner is configured using a `YAML <http://yaml.org/>`_ based
configuration file. Here is an example configuration:

.. code-block:: yaml

    command_queue:
      address: "127.0.0.1:5672"
      queue_name: cortex-dispatcher

    sftp_sources:
      - name: local-test
        address: 127.0.0.1:22
        username: cortex
        regex: "^.*-red\\.xml$"
        directory: /test-data/source
        scan_interval: 2000

    prometheus:
      push_gateway: 127.0.0.1:9091
      push_interval: 5000

    sqlite:
      path: "/var/lib/cortex/cortex.db"
