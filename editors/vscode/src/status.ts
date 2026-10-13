export type ServerStatus = 'starting' | 'running' | 'stopped';

export interface StatusText {
  text: string;
  tooltip: string;
}

export function statusFor(status: ServerStatus, version?: string): StatusText {
  const name = version ? `Craw language server ${version}` : 'Craw language server';
  switch (status) {
    case 'starting':
      return { text: '$(sync~spin) Craw', tooltip: `${name} starting` };
    case 'running':
      return { text: '$(check) Craw', tooltip: `${name} running — click for server actions` };
    case 'stopped':
      return { text: '$(error) Craw', tooltip: `${name} stopped — click to restart` };
  }
}
