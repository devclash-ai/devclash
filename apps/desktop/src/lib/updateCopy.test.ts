import { test } from 'node:test';
import assert from 'node:assert/strict';
import { automaticUpdateDescription, automaticUpdateSummary } from './updateCopy.ts';

test('Windows automatic installation explains closing and reopening without promising a deferred restart', () => {
  const capability = { enabled: true, installMode: 'background' as const, restartRequired: false };
  assert.match(automaticUpdateSummary(capability), /app closes and reopens/);
  assert.match(automaticUpdateDescription(capability), /closes and reopens automatically/);
  assert.doesNotMatch(automaticUpdateDescription(capability), /Restart when|requires your approval|background/);
});
test('Mac and Linux background installation leaves the restart choice to the user', () => {
  const capability = { enabled: true, installMode: 'background' as const, restartRequired: true };
  assert.match(automaticUpdateSummary(capability), /restart when ready/);
  assert.match(automaticUpdateDescription(capability), /background\. Restart when you’re ready/);
  assert.doesNotMatch(automaticUpdateDescription(capability), /closes and reopens/);
});
test('protected installations require explicit approval even while automatic checks are enabled', () => {
  for (const restartRequired of [true, false]) {
    const capability = { enabled: true, installMode: 'manual' as const, restartRequired };
    assert.match(automaticUpdateSummary(capability), /approve installation/);
    assert.match(automaticUpdateDescription(capability), /requires your approval, so choose Install/);
    assert.doesNotMatch(automaticUpdateDescription(capability), /installs new versions automatically/);
  }
});
test('Windows opt out keeps manual installation and its close/reopen behavior visible', () => {
  const description = automaticUpdateDescription({ enabled: false, installMode: 'background', restartRequired: false });
  assert.match(description, /Automatic updates are off/);
  assert.match(description, /check and install/);
  assert.match(description, /Installing an update closes and reopens DevClash/);
});
