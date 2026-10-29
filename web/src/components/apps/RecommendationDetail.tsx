import { ArrowSquareOut, X } from "@phosphor-icons/react";

import type { RecommendationView } from "../../api/apps";
import { appDescription } from "../../lib/appText";
import i18n from "../../i18n";
import { AppIcon } from "./AppIcon";
import { PromptCopy } from "./PromptCopy";

export function RecommendationDetail({
  rec,
  close,
}: {
  rec: RecommendationView;
  close: () => void;
}) {
  const prompt = i18n.t("apps.installPrompt", { name: rec.name, repo: rec.repo });

  return (
    <div className="apps-detail" onPointerDown={(event) => event.stopPropagation()}>
      <div className="apps-detail-head">
        <AppIcon app={rec} size={48} />
        <div className="min-w-0 flex-1">
          <div className="apps-detail-name">{rec.name}</div>
          <div className="apps-detail-meta">
            <span>{i18n.t(`apps.category.${rec.category}`, { defaultValue: rec.category })}</span>
            <span>·</span>
            <span>{i18n.t("apps.externalRecommendation")}</span>
          </div>
        </div>
        <button
          type="button"
          className="apps-icon-button"
          aria-label={i18n.t("apps.close")}
          onClick={close}
        >
          <X size={14} />
        </button>
      </div>

      <p className="apps-detail-description">{appDescription(rec)}</p>

      <div className="apps-links">
        <a className="apps-link" href={rec.repo} target="_blank" rel="noreferrer">
          <ArrowSquareOut size={12} />
          {i18n.t("apps.repo")}
        </a>
      </div>

      <PromptCopy
        title={i18n.t("apps.askAgent")}
        hint={i18n.t("apps.externalRecommendationHint")}
        text={prompt}
      />
    </div>
  );
}
