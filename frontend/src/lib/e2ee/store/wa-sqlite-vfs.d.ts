// wa-sqlite の examples の VFS には型がないので、使う分だけ宣言する。
declare module "@journeyapps/wa-sqlite/src/examples/OPFSCoopSyncVFS.js" {
  export class OPFSCoopSyncVFS {
    static create(name: string, module: unknown): Promise<SQLiteVFS>;
  }
}
