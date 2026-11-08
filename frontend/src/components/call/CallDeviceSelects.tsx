import { useTranslation } from "react-i18next";

import {
  supportsSinkSelection,
  toDeviceOptions,
  useAudioDevices,
} from "../../lib/call/call-devices";

const selectClass =
  "h-9 w-full min-w-0 rounded-md border bg-background px-2 text-sm outline-none focus-visible:ring-3 focus-visible:ring-ring/50";

interface Props {
  micId: string;
  speakerId: string;
  onMicChange: (id: string) => void;
  onSpeakerChange: (id: string) => void;
}

/** マイク/スピーカーの選択。通話バーと設定の通話タブで共用する。 */
export function CallDeviceSelects({ micId, speakerId, onMicChange, onSpeakerChange }: Props) {
  const { t } = useTranslation();
  const devices = useAudioDevices();
  const mics = toDeviceOptions(devices, "audioinput", (n) => t("call.micFallback", { n }));
  const speakers = toDeviceOptions(devices, "audiooutput", (n) => t("call.speakerFallback", { n }));
  const sinkOk = supportsSinkSelection();

  return (
    <div className="space-y-3">
      <label className="block space-y-1">
        <span className="text-xs font-medium text-muted-foreground">{t("call.micLabel")}</span>
        <select
          className={selectClass}
          value={mics.some((d) => d.deviceId === micId) ? micId : ""}
          onChange={(e) => onMicChange(e.target.value)}
        >
          <option value="">{t("call.systemDefault")}</option>
          {mics.map((d) => (
            <option key={d.deviceId} value={d.deviceId}>
              {d.label}
            </option>
          ))}
        </select>
      </label>
      {sinkOk ? (
        <label className="block space-y-1">
          <span className="text-xs font-medium text-muted-foreground">
            {t("call.speakerLabel")}
          </span>
          <select
            className={selectClass}
            value={speakers.some((d) => d.deviceId === speakerId) ? speakerId : ""}
            onChange={(e) => onSpeakerChange(e.target.value)}
          >
            <option value="">{t("call.systemDefault")}</option>
            {speakers.map((d) => (
              <option key={d.deviceId} value={d.deviceId}>
                {d.label}
              </option>
            ))}
          </select>
        </label>
      ) : (
        <p className="text-xs text-muted-foreground">{t("call.speakerUnsupported")}</p>
      )}
    </div>
  );
}
