import { onMount } from "solid-js";
import { connectStream } from "./store";
import { ConnStatus } from "./components/ConnStatus";
import { State } from "./components/State";
import { Emotion } from "./components/Emotion";
import { LookAt } from "./components/LookAt";
import { Audio } from "./components/Audio";
import { Events } from "./components/Events";
import { Sensors } from "./components/Sensors";
import { Settings } from "./components/Settings";
import { TaskHealth } from "./components/TaskHealth";
import { Toast } from "./components/Toast";

export function App() {
  onMount(connectStream);
  return (
    <>
      <main>
        <header>
          <h1>Stack-chan</h1>
          <ConnStatus />
        </header>
        <State />
        <Emotion />
        <LookAt />
        <Audio />
        <Sensors />
        <TaskHealth />
        <Events />
        <Settings />
      </main>
      <Toast />
    </>
  );
}
