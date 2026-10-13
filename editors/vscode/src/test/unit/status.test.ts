import { describe, expect, it } from 'vitest';
import { statusFor } from '../../status';

describe('statusFor', () => {
  it('maps client states to status bar text', () => {
    expect(statusFor('starting')).toEqual({
      text: '$(sync~spin) Craw',
      tooltip: 'Craw language server starting',
    });
    expect(statusFor('running')).toEqual({
      text: '$(check) Craw',
      tooltip: 'Craw language server running — click for server actions',
    });
    expect(statusFor('stopped')).toEqual({
      text: '$(error) Craw',
      tooltip: 'Craw language server stopped — click to restart',
    });
  });

  it('names the server version once it is known', () => {
    expect(statusFor('running', '0.1.0').tooltip).toBe('Craw language server 0.1.0 running — click for server actions');
  });
});
