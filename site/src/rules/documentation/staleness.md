# Staleness

{{#include ../../reference/_rules.md:documentation-staleness}}

## When a finding is right

A finding says a document is a plan whose work is finished, or that a section tells the reader to use a path or script that no longer exists. Code finds the candidates from what Git and the manifests show: release tags, deleted or renamed files, and scripts the manifests do not define. It is right for a plan whose release is tagged and whose paths were removed, or a setup section naming a script that was deleted. It is wrong for outputs a command writes, local or ignored files, examples, and paths the document names as removed.

## Findings it got wrong

All but one of its labeled findings were right, and the one labeled wrong is in one of the maintainer's own repositories, which these pages do not quote, so there is no example here.
