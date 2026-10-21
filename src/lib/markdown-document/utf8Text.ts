const encoder = new TextEncoder();
// BOM 属于磁盘编码层。工作正文中的 U+FEFF 是用户内容，窗口首字符也不能被解码器吞掉。
const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
const LEAF_BYTES = 16 * 1024;
const HASH_BASE = 16777619;

type Node = Leaf | Branch;
interface Summary {
  length: number;
  hash: number;
  height: number;
  newlines: number;
}
interface Leaf extends Summary {
  kind: 'leaf';
  bytes: Uint8Array;
}
interface Branch extends Summary {
  kind: 'branch';
  left: Node;
  right: Node;
}

function pow(exponent: number) {
  let result = 1;
  let base = HASH_BASE;
  while (exponent > 0) {
    if (exponent % 2) result = Math.imul(result, base);
    exponent = Math.floor(exponent / 2);
    base = Math.imul(base, base);
  }
  return result;
}
function leaf(bytes: Uint8Array): Leaf {
  let hash = 0;
  let newlines = 0;
  for (const byte of bytes) {
    hash = (Math.imul(hash, HASH_BASE) + byte + 1) | 0;
    if (byte === 10) newlines++;
  }
  return { kind: 'leaf', bytes, length: bytes.length, hash, height: 1, newlines };
}
function branch(left: Node, right: Node): Branch {
  return {
    kind: 'branch',
    left,
    right,
    length: left.length + right.length,
    hash: (Math.imul(left.hash, pow(right.length)) + right.hash) | 0,
    height: Math.max(left.height, right.height) + 1,
    newlines: left.newlines + right.newlines,
  };
}
function join(left: Node | null, right: Node | null): Node | null {
  if (!left) return right;
  if (!right) return left;
  if (left.height > right.height + 1 && left.kind === 'branch') {
    const tail = join(left.right, right)!;
    if (tail.height > left.left.height + 1 && tail.kind === 'branch')
      return branch(branch(left.left, tail.left), tail.right);
    return branch(left.left, tail);
  }
  if (right.height > left.height + 1 && right.kind === 'branch') {
    const head = join(left, right.left)!;
    if (head.height > right.right.height + 1 && head.kind === 'branch')
      return branch(head.left, branch(head.right, right.right));
    return branch(head, right.right);
  }
  return branch(left, right);
}
function split(node: Node | null, offset: number): [Node | null, Node | null] {
  if (!node) return [null, null];
  if (offset === 0) return [null, node];
  if (offset === node.length) return [node, null];
  if (node.kind === 'leaf')
    return [leaf(node.bytes.subarray(0, offset)), leaf(node.bytes.subarray(offset))];
  if (offset < node.left.length) {
    const [left, right] = split(node.left, offset);
    return [left, join(right, node.right)];
  }
  const [left, right] = split(node.right, offset - node.left.length);
  return [join(node.left, left), right];
}
function* pieces(node: Node | null, from: number, to: number): Generator<Uint8Array> {
  if (!node || from >= to) return;
  if (node.kind === 'leaf') {
    yield node.bytes.subarray(from, to);
    return;
  }
  if (from < node.left.length) yield* pieces(node.left, from, Math.min(to, node.left.length));
  if (to > node.left.length)
    yield* pieces(node.right, Math.max(0, from - node.left.length), to - node.left.length);
}
function fromBytes(bytes: Uint8Array): Node | null {
  let root: Node | null = null;
  for (let start = 0; start < bytes.length; ) {
    let end = Math.min(start + LEAF_BYTES, bytes.length);
    while (end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
    root = join(root, leaf(bytes.subarray(start, end)));
    start = end;
  }
  return root;
}

/** 浏览器/测试端的持久化 UTF-8 Rope。根和叶片可共享，快照及撤销不复制整篇正文。 */
export class Utf8Text {
  private constructor(private readonly root: Node | null) {}
  static from(text: string) {
    // TextEncoder 会静默替换孤立代理项；显式拒绝，不能把有损文本当成成功写入。
    if (/\p{Surrogate}/u.test(text)) throw new Error('正文包含不完整的 Unicode 字符。');
    return new Utf8Text(fromBytes(encoder.encode(text)));
  }
  get byteLength() {
    return this.root?.length ?? 0;
  }
  get lineBreaks() {
    return this.root?.newlines ?? 0;
  }
  private checkRange(from: number, to: number) {
    if (
      !Number.isSafeInteger(from) ||
      !Number.isSafeInteger(to) ||
      from < 0 ||
      from > to ||
      to > this.byteLength
    )
      throw new RangeError(`正文范围越界：${from}..${to}/${this.byteLength}`);
  }
  byteAt(offset: number): number | undefined {
    if (offset < 0 || offset >= this.byteLength) return undefined;
    return pieces(this.root, offset, offset + 1).next().value?.[0];
  }
  isBoundary(offset: number) {
    return (
      Number.isSafeInteger(offset) &&
      offset >= 0 &&
      offset <= this.byteLength &&
      ((this.byteAt(offset) ?? 0) & 0xc0) !== 0x80 &&
      !(this.byteAt(offset - 1) === 13 && this.byteAt(offset) === 10)
    );
  }
  read(from = 0, to = this.byteLength): string {
    this.checkRange(from, to);
    const bytes = new Uint8Array(to - from);
    let offset = 0;
    for (const piece of pieces(this.root, from, to)) {
      bytes.set(piece, offset);
      offset += piece.length;
    }
    return decoder.decode(bytes);
  }
  /** 与原生端一致：范围读取扩大到完整字符/CRLF，调用方按实际返回终点继续读取。 */
  readWindow(fromByte: number, toByte: number) {
    this.checkRange(fromByte, toByte);
    if (toByte - fromByte > 1024 * 1024) throw new RangeError('正文窗口超过 1 MiB。');
    while (fromByte > 0 && !this.isBoundary(fromByte)) fromByte--;
    while (toByte < this.byteLength && !this.isBoundary(toByte)) toByte++;
    return { fromByte, toByte, text: this.read(fromByte, toByte) };
  }
  *chunks(): Generator<string> {
    for (const bytes of pieces(this.root, 0, this.byteLength)) yield decoder.decode(bytes);
  }
  replace(from: number, to: number, text: string) {
    this.checkRange(from, to);
    if (!this.isBoundary(from) || !this.isBoundary(to))
      throw new RangeError('编辑范围切开了 UTF-8 字符或 CRLF。');
    const [left, rest] = split(this.root, from);
    const [, right] = split(rest, to - from);
    return new Utf8Text(join(join(left, Utf8Text.from(text).root), right));
  }
  equals(other: Utf8Text): boolean {
    if (this.root === other.root) return true;
    if (this.byteLength !== other.byteLength || this.root?.hash !== other.root?.hash) return false;
    // 指纹只作快速排除，不能用碰撞概率代替保存完整性判断。
    const a = pieces(this.root, 0, this.byteLength);
    const b = pieces(other.root, 0, other.byteLength);
    let x = a.next();
    let y = b.next();
    let i = 0;
    let j = 0;
    while (!x.done && !y.done) {
      if (x.value[i++] !== y.value[j++]) return false;
      if (i === x.value.length) {
        x = a.next();
        i = 0;
      }
      if (j === y.value.length) {
        y = b.next();
        j = 0;
      }
    }
    return x.done === y.done;
  }
}

export function utf8Length(text: string) {
  return encoder.encode(text).length;
}
/** 一次窗口转换服务多次坐标查询，避免搜索结果逐个重新编码前缀。 */
export function utf8OffsetMap(text: string) {
  const offsets = new Int32Array(text.length + 1);
  let byte = 0;
  for (let index = 0; index < text.length; index++) {
    offsets[index] = byte;
    const code = text.codePointAt(index)!;
    if (code > 0xffff) {
      offsets[++index] = -1;
      byte += 4;
    } else if (code >= 0xd800 && code <= 0xdfff) throw new RangeError('正文包含未配对的代理字符。');
    else byte += code < 0x80 ? 1 : code < 0x800 ? 2 : 3;
  }
  offsets[text.length] = byte;
  return (position: number) => {
    if (
      !Number.isSafeInteger(position) ||
      position < 0 ||
      position > text.length ||
      offsets[position] < 0
    )
      throw new RangeError('UTF-16 位置不是完整字符边界。');
    return offsets[position];
  };
}
export function utf16ToUtf8(text: string, offset: number) {
  if (
    !Number.isSafeInteger(offset) ||
    offset < 0 ||
    offset > text.length ||
    (offset > 0 &&
      /[\uD800-\uDBFF]/.test(text[offset - 1]) &&
      /[\uDC00-\uDFFF]/.test(text[offset] ?? ''))
  )
    throw new RangeError('UTF-16 位置不是完整字符边界。');
  return utf8Length(text.slice(0, offset));
}
export function utf8ToUtf16(text: string, offset: number) {
  const bytes = encoder.encode(text);
  if (
    !Number.isSafeInteger(offset) ||
    offset < 0 ||
    offset > bytes.length ||
    (offset < bytes.length && (bytes[offset] & 0xc0) === 0x80)
  )
    throw new RangeError('UTF-8 位置不是完整字符边界。');
  return decoder.decode(bytes.subarray(0, offset)).length;
}
