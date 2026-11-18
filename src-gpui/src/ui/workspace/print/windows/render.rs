use windows::{
    core::{Error, Interface, Result},
    Data::Pdf::PdfDocument,
    Graphics::Printing::PrintPageDescription,
    Win32::{
        Foundation::{E_FAIL, E_INVALIDARG, POINT},
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::D3D_DRIVER_TYPE_WARP,
            Direct3D11::{D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION},
            Dxgi::{Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGIDevice, IDXGISurface},
        },
        System::WinRT::Pdf::{PdfCreateRenderer, IPdfRendererNative, PDF_RENDER_PARAMS},
    },
};

const WHITE: D2D1_COLOR_F = D2D1_COLOR_F { r: 1., g: 1., b: 1., a: 1. };

pub(super) struct Renderer {
    pub(super) device: ID2D1Device,
    pdf_renderer: IPdfRendererNative,
}

impl Renderer {
    pub(super) fn new() -> Result<Self> {
        unsafe {
            // WARP also works on remote desktops and does not borrow GPUI's render device.
            let mut d3d = None;
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, Default::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None, D3D11_SDK_VERSION, Some(&mut d3d), None, None)?;
            let dxgi: IDXGIDevice = d3d.ok_or(Error::from_hresult(E_FAIL))?.cast()?;
            let factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_MULTI_THREADED, None)?;
            Ok(Self { device: factory.CreateDevice(&dxgi)?, pdf_renderer: PdfCreateRenderer(&dxgi)? })
        }
    }

    pub(super) fn context(&self) -> Result<ID2D1DeviceContext> {
        unsafe { self.device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }
    }

    fn bitmap(context: &ID2D1DeviceContext, w: f32, h: f32, dpi: f32) -> Result<ID2D1Bitmap1> {
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: dpi, dpiY: dpi, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET, ..Default::default()
        };
        let size = D2D_SIZE_U { width: w.round().max(1.) as u32, height: h.round().max(1.) as u32 };
        unsafe { context.CreateBitmap(size, None, 0, &properties) }
    }

    pub(super) fn preview(&self, pdf: &PdfDocument, number: u32, layout: &PrintPageDescription, w: f32, h: f32) -> Result<(IDXGISurface, f32)> {
        let paper = layout.PageSize;
        if !(paper.Width > 0. && paper.Height > 0.) { return Err(Error::from_hresult(E_INVALIDARG)); }
        // Request sizes vary with the preview pane. Keep at most one 2048px preview per callback.
        let scale = (w / paper.Width).min(h / paper.Height).max(0.1).min(2048. / paper.Width.max(paper.Height));
        let dpi = scale * 96.;
        let context = self.context()?;
        let target = Self::bitmap(&context, paper.Width * scale, paper.Height * scale, dpi)?;
        unsafe {
            context.SetTarget(&target);
            context.SetDpi(dpi, dpi);
        }
        self.draw(&context, pdf, number, layout, dpi)?;
        unsafe {
            context.SetTarget(None);
            Ok((target.GetSurface()?, dpi))
        }
    }

    pub(super) fn draw(&self, context: &ID2D1DeviceContext, pdf: &PdfDocument, number: u32, layout: &PrintPageDescription, dpi: f32) -> Result<()> {
        let page = pdf.GetPage(number - 1)?;
        let size = page.Size()?;
        let area = layout.ImageableRect;
        if !(size.Width > 0. && size.Height > 0. && area.Width > 0. && area.Height > 0.) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let fit = (area.Width / size.Width).min(area.Height / size.Height);
        let (w, h) = (size.Width * fit, size.Height * fit);
        let left = area.X + (area.Width - w) / 2.;
        let top = area.Y + (area.Height - h) / 2.;
        let destination = D2D_RECT_F { left, top, right: left + w, bottom: top + h };
        // Bound the raster for a printed page to the long edge of A4 at 300 DPI.
        let scale = (dpi / 96.).min(3508. / w.max(h));
        let bitmap = Self::bitmap(context, w * scale, h * scale, 96.)?;
        unsafe {
            let pixels = bitmap.GetPixelSize();
            let params = PDF_RENDER_PARAMS {
                DestinationWidth: pixels.width, DestinationHeight: pixels.height,
                BackgroundColor: D2D_COLOR_F { r: 1., g: 1., b: 1., a: 1. },
                IgnoreHighContrast: true, ..Default::default()
            };
            self.pdf_renderer.RenderPageToSurface(&page, &bitmap.GetSurface()?, POINT::default(), Some(&params))?;
            context.BeginDraw();
            context.Clear(Some(&WHITE));
            context.DrawBitmap(&bitmap, Some(&destination), 1., D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            context.EndDraw(None, None)?;
        }
        page.Close()
    }
}
