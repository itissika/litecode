type CsvOptions = {
  /** Field separator; one character. Default ",". */
  delimiter?: string;
  /** Quote character; one character. Default is a double quote. */
  quote?: string;
  /** Trim fields while parsing. Default false. */
  trim?: boolean;
};

type HistogramBin = {
  start: number;
  end: number;
  count: number;
};

type DiffLine = {
  kind: "equal" | "insert" | "delete";
  text: string;
};

declare const libs: {
  stats: {
    sum(values: number[]): number;
    mean(values: number[]): number;
    median(values: number[]): number;
    variance(values: number[], sample?: boolean): number;
    stddev(values: number[], sample?: boolean): number;
    quantile(values: number[], q: number): number;
    histogram(values: number[], bins?: number): HistogramBin[];
    min(values: number[]): number;
    max(values: number[]): number;
  };
  csv: {
    parse(text: string, options?: CsvOptions): string[][];
    format(rows: Array<Array<unknown>>, options?: CsvOptions): string;
  };
  text: {
    lines(text: string): string[];
    words(text: string): string[];
    count(text: string, needle: string): number;
    normalizeNewlines(text: string): string;
    truncate(text: string, maxChars: number, suffix?: string): string;
    diffLines(left: string, right: string): DiffLine[];
  };
  data: {
    groupBy<T>(rows: T[], key: string | ((row: T) => unknown)): Record<string, T[]>;
    countBy<T>(rows: T[], key: string | ((row: T) => unknown)): Record<string, number>;
    unique<T>(values: T[]): T[];
    sortBy<T>(rows: T[], key: string | ((row: T) => unknown), direction?: "asc" | "desc"): T[];
  };
};

type FsFailure = { ok: false; reason: "not_found" | "permission" | "io"; message: string };

/** Read-only workspace probes. Synchronous; paths are workspace-relative. */
declare const fs: {
  /** One level, sorted by name, at most 500 entries (message says when cut). */
  ls(path: string): { ok: true; entries: Array<{ name: string; type: "file" | "dir" | "symlink" | "other"; size: number }>; message?: string } | FsFailure;
  /** mtime is ms since the Unix epoch. */
  stat(path: string): { ok: true; type: "file" | "dir" | "symlink" | "other"; size: number; mtime: number } | FsFailure;
  exists(path: string): boolean;
};