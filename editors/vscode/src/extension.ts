import * as path from 'path';
import { workspace, ExtensionContext } from 'vscode';
import { LanguageClient, LanguageClientOptions, ServerOptions, TransportKind } from 'vscode-languageclient/node';

let client: LanguageClient;

export async function activate(context: ExtensionContext) {
    let binaryName = process.platform === 'win32' ? 'craw-lsp.exe' : 'craw-lsp';
    
    let command = process.env.SERVER_PATH;
    if (!command) {
        // 1. Try bundled binary (Release)
        command = context.asAbsolutePath(path.join('bin', binaryName));
        
        // 2. Fallback to workspace target/release (Development)
        if (!require('fs').existsSync(command)) {
            if (workspace.workspaceFolders && workspace.workspaceFolders.length > 0) {
                command = path.join(workspace.workspaceFolders[0].uri.fsPath, 'target', 'release', binaryName);
            }
        }

        // 3. Last resort: target/debug (Legacy/Fallback)
        if (!require('fs').existsSync(command)) {
             if (workspace.workspaceFolders && workspace.workspaceFolders.length > 0) {
                command = path.join(workspace.workspaceFolders[0].uri.fsPath, 'target', 'debug', binaryName);
            }
        }
    }
    
    let serverOptions: ServerOptions = {
        run: { command, transport: TransportKind.stdio },
        debug: { command, transport: TransportKind.stdio }
    };

    let clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'craw' }]
    };

    client = new LanguageClient('crawLanguageServer', 'Craw Language Server', serverOptions, clientOptions);
    await client.start();
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) { return undefined; }
    return client.stop();
}
