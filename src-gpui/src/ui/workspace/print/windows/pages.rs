use super::render::Renderer;
use crate::nomo_mark::session_window::WindowRequest;
use std::sync::{Arc, Mutex};
use windows::{
    core::{implement, Error, IInspectable, Interface, Ref, Result},
    Data::Pdf::PdfDocument,
    Graphics::Printing::{IPrintDocumentSource, IPrintDocumentSource_Impl, PrintPageDescription, PrintTaskOptions},
    Win32::{
        Foundation::{E_ABORT, E_FAIL, E_INVALIDARG},
        Graphics::{Direct2D::Common::D2D_SIZE_F, Imaging::{CLSID_WICImagingFactory2, IWICImagingFactory}, Printing::{FinalPageCount, IPrintPreviewDxgiPackageTarget}},
        Storage::Xps::Printing::IPrintDocumentPackageTarget,
        System::{Com::{CoCreateInstance, CLSCTX_INPROC_SERVER}, WinRT::Printing::*},
    },
};

pub(super) struct Document {
    pdf: PdfDocument,
    count: u32,
    request: WindowRequest,
    error: Mutex<Option<Error>>,
    _directory: tempfile::TempDir,
}

impl Document {
    pub(super) fn new(pdf: PdfDocument, directory: tempfile::TempDir, request: WindowRequest) -> Result<Self> {
        let count = pdf.PageCount()?;
        if count == 0 { return Err(Error::new(E_INVALIDARG, "打印稿没有页面")); }
        Ok(Self { pdf, count, request, error: Mutex::new(None), _directory: directory })
    }
    pub(super) fn check(&self) -> Result<()> {
        if self.request.is_cancelled() { return Err(Error::from_hresult(E_ABORT)); }
        Ok(())
    }
    pub(super) fn last_error(&self) -> Error {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
            .unwrap_or_else(|| Error::new(E_FAIL, "Windows 未能完成打印任务"))
    }
}

#[implement(IPrintDocumentSource, IPrintDocumentPageSource)]
pub(super) struct PageSource(pub(super) Arc<Document>);
impl IPrintDocumentSource_Impl for PageSource_Impl {}

impl IPrintDocumentPageSource_Impl for PageSource_Impl {
    fn GetPreviewPageCollection(&self, target: Ref<IPrintDocumentPackageTarget>) -> Result<IPrintPreviewPageCollection> {
        self.0.check()?;
        let target = unsafe { target.ok()?.GetPackageTarget(&IPrintPreviewDxgiPackageTarget::IID)? };
        Ok(Preview { document: self.0.clone(), target, renderer: Renderer::new()?, layout: Mutex::new(None) }.into())
    }

    fn MakeDocument(&self, options: Ref<IInspectable>, target: Ref<IPrintDocumentPackageTarget>) -> Result<()> {
        let target = target.ok()?;
        let result = self.print(&options.ok()?.cast()?, target);
        if let Err(error) = &result {
            *self.0.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.clone());
            let _ = unsafe { target.Cancel() };
        }
        result
    }
}

impl PageSource_Impl {
    fn print(&self, options: &PrintTaskOptions, target: &IPrintDocumentPackageTarget) -> Result<()> {
        self.0.check()?;
        let renderer = Renderer::new()?;
        let wic: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory2, None, CLSCTX_INPROC_SERVER)? };
        let control = unsafe { renderer.device.CreatePrintControl(&wic, target, None)? };
        let ranges = options.CustomPageRanges()?;
        let mut selected = Vec::new();
        for i in 0..ranges.Size()? {
            let range = ranges.GetAt(i)?;
            selected.push(range.FirstPageNumber()? as u32..=range.LastPageNumber()? as u32);
        }
        for number in 1..=self.0.count {
            if !selected.is_empty() && !selected.iter().any(|range| range.contains(&number)) { continue; }
            self.0.check()?;
            let layout = options.GetPageDescription(number)?;
            let context = renderer.context()?;
            let commands = unsafe { context.CreateCommandList()? };
            unsafe { context.SetTarget(&commands); }
            let dpi = layout.DpiX.max(layout.DpiY).clamp(96, 300) as f32;
            renderer.draw(&context, &self.0.pdf, number, &layout, dpi)?;
            unsafe {
                context.SetTarget(None);
                commands.Close()?;
                control.AddPage(&commands, D2D_SIZE_F { width: layout.PageSize.Width, height: layout.PageSize.Height }, None, None, None)?;
            }
        }
        self.0.check()?;
        unsafe { control.Close() }
    }
}

#[implement(IPrintPreviewPageCollection)]
struct Preview {
    document: Arc<Document>,
    target: IPrintPreviewDxgiPackageTarget,
    renderer: Renderer,
    layout: Mutex<Option<PrintPageDescription>>,
}

impl IPrintPreviewPageCollection_Impl for Preview_Impl {
    fn Paginate(&self, _: u32, options: Ref<IInspectable>) -> Result<()> {
        self.document.check()?;
        let options: PrintTaskOptions = options.ok()?.cast()?;
        *self.layout.lock().unwrap_or_else(|e| e.into_inner()) = Some(options.GetPageDescription(1)?);
        unsafe { self.target.SetJobPageCount(FinalPageCount, self.document.count) }
    }

    fn MakePage(&self, requested: u32, width: f32, height: f32) -> Result<()> {
        self.document.check()?;
        // Windows requests JOB_PAGE_APPLICATION_DEFINED for the initial preview.
        let number = if requested == u32::MAX { 1 } else { requested };
        if !(1..=self.document.count).contains(&number) { return Err(Error::from_hresult(E_INVALIDARG)); }
        let layout = self.layout.lock().unwrap_or_else(|e| e.into_inner())
            .ok_or_else(|| Error::new(E_FAIL, "系统尚未提供打印尺寸"))?;
        let (surface, dpi) = self.renderer.preview(&self.document.pdf, number, &layout, width, height)?;
        unsafe { self.target.DrawPage(number, &surface, dpi, dpi) }
    }
}
