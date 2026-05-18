import { createSignal } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";
import { PosePad } from "./PosePad";

const HOLD_MS = 30_000;

export function LookAt() {
  const [pan, setPan] = createSignal(0);
  const [tilt, setTilt] = createSignal(0);

  const send = async () => {
    try {
      await postJson("/look-at", {
        pan_deg: pan(),
        tilt_deg: tilt(),
        hold_ms: HOLD_MS,
      });
      showToast(`look-at ${pan()}°, ${tilt()}°`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Look-at</h2>
      <PosePad />
      <div class="grid grid-2" style="margin-top:12px">
        <label>
          Pan: <span>{pan()}°</span>
          <input
            type="range"
            min={-60}
            max={60}
            step={1}
            value={pan()}
            onInput={(e) => setPan(Number(e.currentTarget.value))}
            onChange={send}
          />
        </label>
        <label>
          Tilt: <span>{tilt()}°</span>
          <input
            type="range"
            min={-30}
            max={30}
            step={1}
            value={tilt()}
            onInput={(e) => setTilt(Number(e.currentTarget.value))}
            onChange={send}
          />
        </label>
      </div>
      <small>
        Drag the pad for a quick aim; sliders POST on release for fine tweaks. Holds for 30 s. Dashed
        outline shows the servo's actual position when available.
      </small>
    </section>
  );
}
