// Deterministic pure-JavaScript helpers exposed as `libs` to run_code programs.
// Keep this module free of filesystem, network, process, and dynamic import use.

function finiteNumbers(values) {
  if (!Array.isArray(values) || values.length === 0) {
    throw new TypeError('expected a non-empty number array');
  }
  const out = new Array(values.length);
  for (let i = 0; i < values.length; i += 1) {
    const value = values[i];
    if (typeof value !== 'number' || !Number.isFinite(value)) {
      throw new TypeError(`expected a finite number at index ${i}`);
    }
    out[i] = value;
  }
  return out;
}

function sortedNumbers(values) {
  return finiteNumbers(values).slice().sort((a, b) => a - b);
}

function sum(values) {
  let total = 0;
  for (const value of finiteNumbers(values)) total += value;
  return total;
}

function mean(values) {
  return sum(values) / values.length;
}

function median(values) {
  const sorted = sortedNumbers(values);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}

function variance(values, sample = false) {
  const data = finiteNumbers(values);
  if (sample && data.length < 2) throw new RangeError('sample variance needs at least two values');
  const average = sum(data) / data.length;
  let total = 0;
  for (const value of data) total += (value - average) ** 2;
  return total / (sample ? data.length - 1 : data.length);
}

function stddev(values, sample = false) {
  return Math.sqrt(variance(values, sample));
}

function quantile(values, q) {
  if (typeof q !== 'number' || !Number.isFinite(q) || q < 0 || q > 1) {
    throw new RangeError('quantile requires q in [0, 1]');
  }
  const sorted = sortedNumbers(values);
  if (sorted.length === 1) return sorted[0];
  const position = (sorted.length - 1) * q;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  if (lower === upper) return sorted[lower];
  const weight = position - lower;
  return sorted[lower] * (1 - weight) + sorted[upper] * weight;
}

function histogram(values, bins = 10) {
  const data = finiteNumbers(values);
  if (!Number.isInteger(bins) || bins < 1) throw new RangeError('histogram bins must be a positive integer');
  let low = data[0];
  let high = data[0];
  for (const value of data) {
    if (value < low) low = value;
    if (value > high) high = value;
  }
  if (low === high) return [{ start: low, end: high, count: data.length }];
  const width = (high - low) / bins;
  const counts = new Array(bins).fill(0);
  for (const value of data) {
    const index = Math.min(bins - 1, Math.floor((value - low) / width));
    counts[index] += 1;
  }
  return counts.map((count, index) => ({
    start: low + width * index,
    end: index === bins - 1 ? high : low + width * (index + 1),
    count,
  }));
}

function csvOptions(options = {}) {
  if (options === null || typeof options !== 'object' || Array.isArray(options)) {
    throw new TypeError('CSV options must be an object');
  }
  const delimiter = options.delimiter ?? ',';
  const quote = options.quote ?? '"';
  const trim = options.trim ?? false;
  if (typeof delimiter !== 'string' || delimiter.length !== 1) throw new TypeError('CSV delimiter must be one character');
  if (typeof quote !== 'string' || quote.length !== 1) throw new TypeError('CSV quote must be one character');
  if (delimiter === quote) throw new RangeError('CSV delimiter and quote must differ');
  if (typeof trim !== 'boolean') throw new TypeError('CSV trim must be boolean');
  return { delimiter, quote, trim };
}

function csvParse(text, options) {
  if (typeof text !== 'string') throw new TypeError('CSV input must be a string');
  const { delimiter, quote, trim } = csvOptions(options);
  const rows = [];
  let row = [];
  let field = '';
  let quoted = false;
  let atFieldStart = true;
  const finishField = () => {
    row.push(trim ? field.trim() : field);
    field = '';
    atFieldStart = true;
  };
  const finishRow = () => {
    finishField();
    rows.push(row);
    row = [];
  };
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (quoted) {
      if (ch === quote) {
        if (text[i + 1] === quote) {
          field += quote;
          i += 1;
        } else {
          quoted = false;
        }
      } else {
        field += ch;
      }
      continue;
    }
    if (ch === quote && atFieldStart) {
      quoted = true;
      atFieldStart = false;
    } else if (ch === delimiter) {
      finishField();
    } else if (ch === '\n' || ch === '\r') {
      finishRow();
      if (ch === '\r' && text[i + 1] === '\n') i += 1;
    } else {
      field += ch;
      atFieldStart = false;
    }
  }
  if (quoted) throw new SyntaxError('unterminated quoted CSV field');
  if (field.length > 0 || row.length > 0 || (text.length > 0 && text[text.length - 1] === delimiter)) {
    finishRow();
  }
  return rows;
}

function csvFormat(rows, options) {
  if (!Array.isArray(rows)) throw new TypeError('CSV rows must be an array');
  const { delimiter, quote } = csvOptions(options);
  return rows.map((row) => {
    if (!Array.isArray(row)) throw new TypeError('each CSV row must be an array');
    return row.map((value) => {
      const text = value === null || value === undefined ? '' : String(value);
      const mustQuote = text.includes(delimiter) || text.includes(quote) || text.includes('\n') || text.includes('\r');
      return mustQuote ? quote + text.split(quote).join(quote + quote) + quote : text;
    }).join(delimiter);
  }).join('\n');
}

function textLines(text) {
  if (typeof text !== 'string') throw new TypeError('text must be a string');
  if (text.length === 0) return [];
  return text.split(/\r\n|\n|\r/);
}

function textWords(text) {
  if (typeof text !== 'string') throw new TypeError('text must be a string');
  return text.match(/\S+/g) ?? [];
}

function textCount(text, needle) {
  if (typeof text !== 'string' || typeof needle !== 'string' || needle.length === 0) {
    throw new TypeError('count requires non-empty strings');
  }
  let count = 0;
  let offset = 0;
  for (;;) {
    const found = text.indexOf(needle, offset);
    if (found < 0) return count;
    count += 1;
    offset = found + needle.length;
  }
}

function normalizeNewlines(text) {
  if (typeof text !== 'string') throw new TypeError('text must be a string');
  return text.replace(/\r\n?/g, '\n');
}

function truncate(text, maxChars, suffix = '…') {
  if (typeof text !== 'string' || typeof suffix !== 'string') throw new TypeError('truncate requires strings');
  if (!Number.isInteger(maxChars) || maxChars < 0) throw new RangeError('maxChars must be a non-negative integer');
  if (text.length <= maxChars) return text;
  return text.slice(0, maxChars) + suffix;
}

function diffLines(left, right) {
  const a = textLines(left);
  const b = textLines(right);
  if (a.length > 1000 || b.length > 1000) {
    throw new RangeError('diffLines supports at most 1000 lines per side; slice the input first');
  }
  const width = b.length + 1;
  const table = new Uint32Array((a.length + 1) * width);
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) {
      table[i * width + j] = a[i] === b[j]
        ? table[(i + 1) * width + j + 1] + 1
        : Math.max(table[(i + 1) * width + j], table[i * width + j + 1]);
    }
  }
  const output = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      output.push({ kind: 'equal', text: a[i] });
      i += 1;
      j += 1;
    } else if (table[(i + 1) * width + j] >= table[i * width + j + 1]) {
      output.push({ kind: 'delete', text: a[i] });
      i += 1;
    } else {
      output.push({ kind: 'insert', text: b[j] });
      j += 1;
    }
  }
  while (i < a.length) output.push({ kind: 'delete', text: a[i++] });
  while (j < b.length) output.push({ kind: 'insert', text: b[j++] });
  return output;
}

function keyOf(row, key) {
  if (typeof key === 'function') return key(row);
  if (row !== null && typeof row === 'object') return row[key];
  return undefined;
}

function groupBy(rows, key) {
  if (!Array.isArray(rows)) throw new TypeError('groupBy rows must be an array');
  if (typeof key !== 'string' && typeof key !== 'function') throw new TypeError('groupBy key must be a string or function');
  const groups = Object.create(null);
  for (const row of rows) {
    const name = String(keyOf(row, key));
    if (!Object.hasOwn(groups, name)) groups[name] = [];
    groups[name].push(row);
  }
  return groups;
}

function countBy(rows, key) {
  const groups = groupBy(rows, key);
  const counts = Object.create(null);
  for (const name of Object.keys(groups)) counts[name] = groups[name].length;
  return counts;
}

function unique(values) {
  if (!Array.isArray(values)) throw new TypeError('unique values must be an array');
  return [...new Set(values)];
}

function sortBy(rows, key, direction = 'asc') {
  if (!Array.isArray(rows)) throw new TypeError('sortBy rows must be an array');
  if (typeof key !== 'string' && typeof key !== 'function') throw new TypeError('sortBy key must be a string or function');
  if (direction !== 'asc' && direction !== 'desc') throw new RangeError('sortBy direction must be asc or desc');
  const factor = direction === 'asc' ? 1 : -1;
  return rows.slice().sort((left, right) => {
    const a = keyOf(left, key);
    const b = keyOf(right, key);
    if (a === b) return 0;
    if (a === null || a === undefined) return 1;
    if (b === null || b === undefined) return -1;
    return (a < b ? -1 : 1) * factor;
  });
}

function makeLibs() {
  return Object.freeze({
    stats: Object.freeze({
      sum, mean, median, variance, stddev, quantile, histogram,
      min: (values) => sortedNumbers(values)[0],
      max: (values) => sortedNumbers(values).at(-1),
    }),
    csv: Object.freeze({ parse: csvParse, format: csvFormat }),
    text: Object.freeze({
      lines: textLines,
      words: textWords,
      count: textCount,
      normalizeNewlines,
      truncate,
      diffLines,
    }),
    data: Object.freeze({ groupBy, countBy, unique, sortBy }),
  });
}
