Search file contents with a regular expression, case-sensitive by default. Returns matching lines with line numbers, grouped by file. A few code hits come back as the code enclosing them, which often answers what several read calls would.
A hit list too wide for one response opens with the files that match most, ranked by hit count, and carries the lines that fit; the matches left over are written to a workspace file that read pages through with start_line and end_line. Narrow path or glob instead of trying to raise a limit.
Example: {"pattern":"fn main","path":"src"}
