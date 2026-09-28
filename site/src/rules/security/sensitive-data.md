# Sensitive data

{{#include ../../reference/_rules.md:security-sensitive-data}}

## When a finding is right

A finding says a function writes a password, token, key or personal data to a log, or sends internal error details to a remote client. It is right when a secret reaches a log, or when the text of a database or library error reaches someone outside the service, such as an API that returns an exception's message to its users. It is wrong when only the operator or the person running the program reads the output, when the error text is a message the program wrote itself, or when the caller is the project's own service. Of the 126 findings labeled wrong or debatable so far outside Bend 2 code, 23 were read only by a local user, 20 were messages the program wrote and 16 went to the project's own services.

An error-detail finding is asked who reads the error text, with the opening of the root README, and a log line that runs only when an operator turns on a setting meant for logging those values is a note.

## Findings it got wrong

Labeled wrong by reading the code, on open-source projects the rules were tuned on.

<!-- example: linkace 7317edba939414002706b184b114da176178f0286816898476f110a80c070009 -->
### LinkAce: `viewBackupCodes`

- **Where:** [`app/Console/Commands/ViewRecoveryCodesCommand.php:33`](https://github.com/Kovah/LinkAce/blob/d6821661fb5878850738dc5f3d3593799d89445f/app/Console/Commands/ViewRecoveryCodesCommand.php#L33) in Kovah/LinkAce at `d682166`.
- **Finding (review):** `ViewRecoveryCodesCommand::viewBackupCodes` writes a password, token, key or personal data to a log.
- **Why it was wrong:** `2fa:view-recovery-codes` is an admin command whose purpose is to show a locked-out user's recovery codes. `$this->line($code)` prints them to the operator's terminal, not to a log; printing an identifier instead would defeat the command.
- **Since:** cleared in 0.21.0, which tells values a command-line tool shows its operator on purpose from what a log keeps ([changelog](../../changelog.md#0210---2026-09-26)).

<!-- example: wtf f866de06fd20050fab0940b792d5fd01b86a742ef47abb643a0c9073f9d7b781 -->
### WTF Dial: `handleDialIndex`

- **Where:** [`http/dial.go:66`](https://github.com/benbjohnson/wtf/blob/05bc90c940d5f9e2490fc93cf467d9e8aa48ad63/http/dial.go#L66) in benbjohnson/wtf at `05bc90c`.
- **Finding (consider):** `Server::handleDialIndex` puts the text of a library or database error into an error message, which likely reaches a remote client.
- **Why it was wrong:** The error goes to the project's central `Error` helper, which sends the client a message the program wrote (such as "Dial not found.") or "Internal error." for any other error, and logs and reports internal errors. No SQLite error text reaches the client.
- **Since:** a note since 0.20.0: such a finding, in a function whose error message carries another error's text, is a note, since a central handler often replaces that text; 1 of 28 such considers labeled was right ([changelog](../../changelog.md#0200---2026-09-26)).

<!-- example: wild-workouts 688e7b873fd2c6b6719711d24f212a30eb2ccdd5dac9a40d4f123ee831f1e26a -->
### Wild Workouts: `MakeHourAvailable`

- **Where:** [`internal/trainer/ports/grpc.go:29`](https://github.com/ThreeDotsLabs/wild-workouts-go-ddd-example/blob/8ecfcdf05b1462c4757bd2dcac9086c78e9f7791/internal/trainer/ports/grpc.go#L29) in ThreeDotsLabs/wild-workouts-go-ddd-example at `8ecfcdf`.
- **Finding (review):** `GrpcServer::MakeHourAvailable` sends internal error details to a remote client.
- **Why it was wrong:** The gRPC caller is the project's own trainings service: the gRPC services accept only authenticated invokers, and the public HTTP port turns such errors into a generic "Internal server error". The text never reaches an outside party.
- **Since:** not addressed; reported the same way from 0.19.0 through 0.25.0. Errors returned between a project's own services are a known weak spot of this rule.
