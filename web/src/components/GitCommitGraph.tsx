import {
  GIT_GRAPH_ROW_HEIGHT,
  graphWidth,
  laneX,
  type GitGraphLayout,
  type GraphRow,
} from "../lib/gitGraph";

const LANE_COLORS = 7;

function laneClass(lane: number): string {
  return `git-graph-lane-${lane % LANE_COLORS}`;
}

function mergePath(row: number, from: number, to: number, maxLanes: number): string {
  const y0 = row * GIT_GRAPH_ROW_HEIGHT;
  const yMid = y0 + GIT_GRAPH_ROW_HEIGHT / 2;
  const y1 = y0 + GIT_GRAPH_ROW_HEIGHT;
  const x0 = laneX(from, maxLanes);
  const x1 = laneX(to, maxLanes);
  if (x0 === x1) return `M ${x0} ${yMid} L ${x1} ${y1}`;
  return `M ${x0} ${yMid} Q ${x1} ${yMid} ${x1} ${y1}`;
}

function RowLines({
  row,
  index,
  maxLanes,
}: {
  row: GraphRow;
  index: number;
  maxLanes: number;
}) {
  const y0 = index * GIT_GRAPH_ROW_HEIGHT;
  const yMid = y0 + GIT_GRAPH_ROW_HEIGHT / 2;
  const y1 = y0 + GIT_GRAPH_ROW_HEIGHT;
  const x = laneX(row.lane, maxLanes);

  return (
    <g>
      {row.passing.map((lane) => {
        const px = laneX(lane, maxLanes);
        return (
          <line
            key={`pass-${lane}`}
            className={laneClass(lane)}
            x1={px}
            y1={y0}
            x2={px}
            y2={y1}
          />
        );
      })}
      {row.incoming && (
        <line className={laneClass(row.lane)} x1={x} y1={y0} x2={x} y2={yMid} />
      )}
      {row.continueDown && (
        <line className={laneClass(row.lane)} x1={x} y1={yMid} x2={x} y2={y1} />
      )}
      {row.merges.map((m, i) => (
        <path
          key={`m-${i}`}
          className={laneClass(m.to)}
          d={mergePath(index, m.from, m.to, maxLanes)}
          fill="none"
        />
      ))}
      <circle
        className={laneClass(row.lane)}
        cx={x}
        cy={yMid}
        r={index === 0 ? 3.25 : 2.75}
      />
    </g>
  );
}

export function GitCommitGraph({
  layout,
  maxLanes,
}: {
  layout: GitGraphLayout;
  maxLanes: number;
}) {
  if (layout.rows.length === 0) return null;
  const width = graphWidth(layout.laneCount, maxLanes);
  const height = layout.rows.length * GIT_GRAPH_ROW_HEIGHT;

  return (
    <svg
      className="git-commit-graph pointer-events-none absolute top-0 left-0"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      aria-hidden
    >
      {layout.rows.map((row, index) => (
        <RowLines key={row.sha} row={row} index={index} maxLanes={maxLanes} />
      ))}
    </svg>
  );
}
