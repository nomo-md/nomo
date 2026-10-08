declare module 'hypher' {
  export interface HyphenationPatterns {
    leftmin: number;
    rightmin: number;
    patterns: Record<string, string>;
    exceptions?: string;
  }
  export default class Hypher {
    constructor(patterns: HyphenationPatterns);
    hyphenate(word: string): string[];
  }
}

declare module 'hyphenation.en-us' {
  import type { HyphenationPatterns } from 'hypher';
  const patterns: HyphenationPatterns;
  export default patterns;
}
