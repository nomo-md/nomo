using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;

// 仅测试进程使用：短时采集前台客户区，不截取其他窗口，像素流有 128 MiB 硬上限。
public sealed class OpeningCapture : IDisposable {
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref PointNative p);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(PointNative p);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flags);
    [DllImport("user32.dll")] static extern uint SendInput(uint n, Input[] data, int size);
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int left, top, right, bottom; }
    [StructLayout(LayoutKind.Sequential)] struct PointNative { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint type; public Union u; }
    [StructLayout(LayoutKind.Explicit)] struct Union {
        [FieldOffset(0)] public Keyboard keyboard;
        [FieldOffset(0)] public Mouse mouse;
    }
    [StructLayout(LayoutKind.Sequential)] struct Keyboard { public ushort vk, scan; public uint flags, time; public UIntPtr extra; }
    [StructLayout(LayoutKind.Sequential)] struct Mouse { public int x,y; public uint data,flags,time; public UIntPtr extra; }
    public static double Now() { return (double)Stopwatch.GetTimestamp() * 1000.0 / Stopwatch.Frequency; }
    readonly object gate = new object();
    readonly List<double[]> frames = new List<double[]>();
    volatile bool stopping;
    Thread worker;
    string failure, rawPath;
    IntPtr window;
    uint pid;
    int left, top, fullWidth, fullHeight, width, height;
    int[] region;
    byte[] stableReference;
    double stableSince, stableThrough;
    public object SetRegion(double x, double y, double w, double h, double scale) {
        double sx = (double)width / fullWidth * scale, sy = (double)height / fullHeight * scale;
        int x0=(int)Math.Ceiling((x+8)*sx), y0=(int)Math.Ceiling((y+8)*sy);
        int x1=(int)Math.Floor((x+w-12)*sx), y1=(int)Math.Floor((y+h-8)*sy);
        if (x0<0 || y0<0 || x1>width || y1>height || x1-x0<40 || y1-y0<40)
            throw new InvalidOperationException("capture region outside client area");
        var points = new List<int>();
        for (int row=y0; row<y1; row++) for (int col=x0; col<x1; col++) points.Add(row*width+col);
        lock (gate) { region=points.ToArray(); stableReference=null; stableSince=stableThrough=0; }
        return new { pixels=points.Count };
    }
    public object Start(int processId, string path) {
        if (worker != null) throw new InvalidOperationException("capture already started");
        pid = (uint)processId;
        window = GetForegroundWindow();
        GetWindowThreadProcessId(window, out uint actual);
        if (actual != pid) throw new InvalidOperationException("test window is not foreground");
        if (!GetClientRect(window, out Rect r)) throw new InvalidOperationException("client rect unavailable");
        var origin = new PointNative();
        if (!ClientToScreen(window, ref origin)) throw new InvalidOperationException("client origin unavailable");
        left = origin.x; top = origin.y; fullWidth = r.right; fullHeight = r.bottom;
        if (fullWidth < 320 || fullHeight < 200 || fullWidth > 3840 || fullHeight > 2160)
            throw new InvalidOperationException("unsupported capture dimensions");
        width = (fullWidth + 3) / 4; height = (fullHeight + 3) / 4;
        rawPath = Path.GetFullPath(path);
        CheckWindow();
        worker = new Thread(CaptureLoop) { IsBackground = true };
        worker.Start();
        return new { fullWidth, fullHeight, width, height, intervalMs = 25, maximumBytes = 134217728 };
    }
    void CheckWindow() {
        if (GetForegroundWindow() != window) throw new InvalidOperationException("foreground window changed");
        GetWindowThreadProcessId(window, out uint actual);
        var origin = new PointNative();
        if (actual != pid || !GetClientRect(window, out Rect r) || !ClientToScreen(window, ref origin)
            || origin.x != left || origin.y != top || r.right != fullWidth || r.bottom != fullHeight)
            throw new InvalidOperationException("test window moved, resized or disappeared");
        foreach (var p in new [] {
            new PointNative {x=left+8,y=top+8}, new PointNative {x=left+fullWidth-9,y=top+8},
            new PointNative {x=left+8,y=top+fullHeight-9}, new PointNative {x=left+fullWidth-9,y=top+fullHeight-9},
            new PointNative {x=left+fullWidth/2,y=top+fullHeight/2}})
            if (GetAncestor(WindowFromPoint(p), 2) != window)
                throw new InvalidOperationException("test client area is occluded or outside the desktop");
    }
    void CaptureLoop() {
        try {
            using (var output = new FileStream(rawPath, FileMode.CreateNew, FileAccess.Write, FileShare.Read))
            using (var full = new Bitmap(fullWidth, fullHeight, PixelFormat.Format24bppRgb))
            using (var small = new Bitmap(width, height, PixelFormat.Format24bppRgb))
            using (var screen = Graphics.FromImage(full))
            using (var scaled = Graphics.FromImage(small)) {
                scaled.InterpolationMode = InterpolationMode.Bilinear;
                var pixels = new byte[((width * 3 + 3) & ~3) * height];
                var gray = new byte[width * height];
                var started = Now();
                while (!stopping) {
                    double begin = Now();
                    if (begin - started > 45000 || output.Position + gray.Length > 134217728)
                        throw new InvalidOperationException("bounded capture budget exceeded");
                    CheckWindow();
                    screen.CopyFromScreen(left, top, 0, 0, full.Size, CopyPixelOperation.SourceCopy);
                    scaled.DrawImage(full, new Rectangle(0, 0, width, height));
                    var bits = small.LockBits(new Rectangle(0, 0, width, height), ImageLockMode.ReadOnly, PixelFormat.Format24bppRgb);
                    try {
                        Marshal.Copy(bits.Scan0, pixels, 0, pixels.Length);
                        for (int y=0; y<height; y++) for (int x=0; x<width; x++) {
                            int at = y * bits.Stride + x * 3;
                            gray[y*width+x] = (byte)((pixels[at]*29 + pixels[at+1]*150 + pixels[at+2]*77) >> 8);
                        }
                    } finally { small.UnlockBits(bits); }
                    CheckWindow();
                    double end = Now();
                    output.Write(gray, 0, gray.Length);
                    lock (gate) {
                        frames.Add(new [] {begin, end});
                        if (region != null) {
                            bool changed = stableReference == null;
                            // 比离线完整画面对比更严格，避免逐帧微小变化累积成假稳定。
                            int allowed = Math.Max(5, (int)Math.Ceiling(region.Length*.0005))/2, different=0;
                            if (!changed) foreach (int p in region)
                                if (Math.Abs(gray[p]-stableReference[p])>6 && ++different>allowed) {changed=true;break;}
                            if (changed) {stableReference=(byte[])gray.Clone(); stableSince=end;}
                            stableThrough=end;
                        }
                    }
                    int delay = (int)Math.Max(1, 25 - (Now() - begin));
                    Thread.Sleep(delay);
                }
            }
        } catch (Exception error) { failure = error.Message; }
    }
    public object Mark() {
        if (failure != null) throw new InvalidOperationException(failure);
        lock (gate) return new { clock = Now(), index = frames.Count - 1, stableMs=stableThrough-stableSince };
    }
    public object TypeProbe() {
        CheckWindow();
        double at = Now();
        foreach (uint flags in new uint[] {4,6}) {
            var input = new Input {type=1,u=new Union {keyboard=new Keyboard {scan=(ushort)'Q',flags=flags}}};
            if (SendInput(1, new [] {input}, Marshal.SizeOf(typeof(Input))) != 1)
                throw new InvalidOperationException("SendInput failed");
        }
        return new { clock = at, text = "Q" };
    }
    public object Stop() {
        stopping = true;
        if (worker != null && !worker.Join(5000)) throw new InvalidOperationException("capture worker did not stop");
        lock (gate) return new { failure, frames = frames.ToArray(), rawPath, width, height, fullWidth, fullHeight };
    }
    public void SaveFrame(int index, string path) {
        if (!stopping) throw new InvalidOperationException("stop capture before saving evidence");
        int count; lock (gate) count=frames.Count;
        if (index < 0 || index >= count) throw new ArgumentOutOfRangeException("index");
        var gray = new byte[width * height];
        using (var input = File.OpenRead(rawPath)) {
            input.Position = (long)index * gray.Length;
            input.ReadExactly(gray);
        }
        using (var bitmap = new Bitmap(width, height, PixelFormat.Format24bppRgb)) {
            for (int y=0; y<height; y++) for (int x=0; x<width; x++) {
                int c=gray[y*width+x]; bitmap.SetPixel(x,y,Color.FromArgb(c,c,c));
            }
            bitmap.Save(path, ImageFormat.Png);
        }
    }
    public void Dispose() { Stop(); }
}
