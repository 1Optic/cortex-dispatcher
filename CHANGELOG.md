# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.0] - 2026-03-16

### Added

- Manage database with Refinery migrations

### Changed

- Switch database from PostgreSQL to SQLite

## [1.7.0] - 2025-02-09

### Added

- Added deduplication by file name

## [1.6.1] - 2024-10-29

### Fixed

- Use Debian Trixie for Docker image to fix OpenSSL error

## [1.6.0] - 2024-09-12

### Added

- Add TLS implementation

### Changed

- Upgrade dependencies

## [1.5.0] - 2024-09-03

### Added

- Option for specifying dev-stack root directory.

### Fixed

- Fix handling of stop signal (SIGTERM) for clean shutdown.

[1.5.0]: https://gitlab.1optic.io/hitc/cortex-dispatcher/-/compare/1.4.0...1.5.0
[1.6.0]: https://gitlab.1optic.io/hitc/cortex-dispatcher/-/compare/1.5.0...1.6.0
[1.6.1]: https://gitlab.1optic.io/hitc/cortex-dispatcher/-/compare/1.6.0...1.6.1
[1.7.0]: https://gitlab.1optic.io/hitc/cortex-dispatcher/-/compare/1.6.1...1.7.0
[2.0.0]: https://gitlab.1optic.io/hitc/cortex-dispatcher/-/compare/1.7.0...2.0.0
[Unreleased]: https://gitlab.1optic.io/hitc/cortex-dispatcher/-/compare/2.0.0...HEAD
