import { createWritingStatsEngine } from './writingStatsEngine';
import type { StatsRequest } from './writingStatsProtocol';

const calculate = createWritingStatsEngine();
self.onmessage = (event: MessageEvent<StatsRequest>) => self.postMessage(calculate(event.data));
