import { describe, expect, it } from 'vitest';
import { filterPromptNoise } from './prompt';

describe('filterPromptNoise', () => {
  it('replaces an appshot block with its app name', () => {
    const content = `Before\n\n<appshot app="Xcode" bundle-identifier="com.apple.dt.Xcode" window-title="WindowBackgroundPicker.swift" image="shot.png">
Window: "WindowBackgroundPicker.swift", App: Xcode.
lots of captured UI text
</appshot>\n\nAfter`;

    expect(filterPromptNoise(content)).toBe(
      'Before\n\nUser Included an appshot of Xcode\n\nAfter'
    );
  });

  it('replaces multiple appshots and supports single-quoted or self-closing tags', () => {
    const content = `<appshot app='Xcode'>details</appshot> and <appshot app="Safari" />`;

    expect(filterPromptNoise(content)).toBe(
      'User Included an appshot of Xcode and User Included an appshot of Safari'
    );
  });

  it('removes internal prompt noise blocks', () => {
    const content = `Before

<fork-boilerplate>worker instructions</fork-boilerplate>
<command-name>/model</command-name>
<command-message>model</command-message>
<command-args></command-args>
<local-command-stdout>Set model</local-command-stdout>
<task-notification><summary>Worker finished</summary><usage>tokens</usage></task-notification>
<usage><subagent_tokens>42</subagent_tokens></usage>
<file path="example.swift">internal file payload</file>
<packetEnd>internal packet metadata</packetEnd>

After`;

    expect(filterPromptNoise(content)).toBe('Before\n\n\nAfter');
  });

  it('removes isolated noise markers', () => {
    expect(filterPromptNoise('Before <file> and <packetEnd> after')).toBe('Before  and  after');
  });

  it('leaves prompts without appshots unchanged', () => {
    expect(filterPromptNoise('Please fix the layout.')).toBe('Please fix the layout.');
  });
});
