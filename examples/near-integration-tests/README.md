# NEAR sandbox integration tests

These host-only tests sign through Gnorix core and submit the shared NEAR wire
types to the compiled example contract in a local nearcore 2.14.1 sandbox.
They intentionally use synthetic
candidate bytes to test the authorization protocol independently of biometrics.

See [the counter example](../near-contract/README.md) for build prerequisites,
commands, contract semantics, and tested rejection cases.
