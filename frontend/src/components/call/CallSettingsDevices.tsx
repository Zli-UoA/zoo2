import { useCall } from "../../lib/call/call-context";
import { CallDeviceSelects } from "./CallDeviceSelects";

/** 設定の通話タブ: 既定のマイク/スピーカー。通話中なら即時に反映する。 */
export function CallSettingsDevices() {
  const { selectedMicId, selectedSpeakerId, setMicrophone, setSpeaker } = useCall();
  return (
    <CallDeviceSelects
      micId={selectedMicId}
      speakerId={selectedSpeakerId}
      onMicChange={(id) => {
        void setMicrophone(id);
      }}
      onSpeakerChange={(id) => {
        void setSpeaker(id);
      }}
    />
  );
}
