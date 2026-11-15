/**
 * 鍵バックアップからの取り込み（docs/e2ee.md の「新しい端末での解錠」の手順 5）の順番待ちと試し直し。
 *
 * 暗号ストアを開いたタブ（担当）で 1 つずつ行う。実行中に頼まれたら、終わったあとにもう一度行う。
 * 実行中に担当が替わって頼まれた分は、今の担当で行う。通信の失敗などで止まったら、間隔をあけて
 * （`retryMs` から倍にし、最大 `retryMaxMs`）試し直す。取り込みは進み具合を保存するので、試し直しは
 * 続きのページから読む。試し直しをやめるのは、成功したとき（取り込み終えた、版を使えない、版が
 * 消えたと分かった）と、担当が替わったとき（新しい担当の起動時の確認から頼み直す）。
 */
export class BackupImportQueue<T extends object> {
  private running = false;
  private again = false;
  private retry: ReturnType<typeof setTimeout> | null = null;
  private delay: number;

  constructor(
    private readonly options: {
      /** 今の担当。暗号ストアを開いていなければ `null`。 */
      current: () => T | null;
      /** `target` で取り込む。失敗したら投げる。 */
      run: (target: T) => Promise<unknown>;
      retryMs: number;
      retryMaxMs: number;
    },
  ) {
    this.delay = options.retryMs;
  }

  /** 取り込みを頼む。担当がなければ何もしない。 */
  request(): void {
    const target = this.options.current();
    if (!target) return;
    this.clearRetry();
    if (this.running) {
      this.again = true;
      return;
    }
    this.running = true;
    void this.drain(target);
  }

  /** 担当が暗号ストアを手放した。試し直しの予定と間隔を戻す。 */
  reset(): void {
    this.clearRetry();
    this.delay = this.options.retryMs;
  }

  private async drain(target: T): Promise<void> {
    let failed = false;
    try {
      do {
        this.again = false;
        try {
          await this.options.run(target);
          failed = false;
        } catch {
          failed = true;
        }
      } while (this.again && this.options.current() === target);
    } finally {
      this.running = false;
      const current = this.options.current();
      if (!failed) {
        this.delay = this.options.retryMs;
      } else if (current === target) {
        const delay = this.delay;
        this.delay = Math.min(delay * 2, this.options.retryMaxMs);
        this.retry = setTimeout(() => {
          this.retry = null;
          if (this.options.current() === target) this.request();
        }, delay);
      }
      // 実行中に担当が替わり、新しい担当から頼まれた分は、今の担当で行う。
      if (this.again && current && current !== target) this.request();
    }
  }

  private clearRetry(): void {
    if (this.retry) clearTimeout(this.retry);
    this.retry = null;
  }
}
