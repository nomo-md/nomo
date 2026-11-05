#include "ShellHelpers.h"

#include <Windows.h>
#include <cwctype>

namespace {

bool StartsWith(std::wstring_view value, std::wstring_view prefix) noexcept {
    if (value.size() < prefix.size()) { return false; }
    for (std::size_t index = 0; index < prefix.size(); ++index) {
        if (std::towlower(value[index]) != prefix[index]) { return false; }
    }
    return true;
}

}  // namespace

bool IsMarkdownPath(std::wstring_view path) noexcept {
    const std::size_t separator = path.find_last_of(L"\\/");
    const std::size_t dot = path.find_last_of(L'.');
    if (dot == std::wstring_view::npos ||
        (separator != std::wstring_view::npos && dot < separator)) {
        return false;
    }

    const auto extension = path.substr(dot);
    return (extension.size() == 3 && StartsWith(extension, L".md")) ||
           (extension.size() == 9 && StartsWith(extension, L".markdown"));
}

bool IsNomoSelectionSupported(
    NomoCommandKind kind,
    const std::vector<NomoSelectionItem>& items) noexcept {
    if (kind == NomoCommandKind::Markdown) {
        if (items.empty() || items.size() > NomoMaximumSelection) {
            return false;
        }
        for (const NomoSelectionItem& item : items) {
            if (item.isFolder || !IsMarkdownPath(item.path)) {
                return false;
            }
        }
        return true;
    }

    // Explorer passes no selected item for a folder-background command.
    return items.empty() || (items.size() == 1 && items.front().isFolder);
}

std::wstring QuoteCommandLineArgument(std::wstring_view argument) {
    if (!argument.empty() &&
        argument.find_first_of(L" \t\n\v\"") == std::wstring_view::npos) {
        return std::wstring(argument);
    }

    std::wstring result;
    result.push_back(L'"');
    std::size_t backslashes = 0;
    for (wchar_t character : argument) {
        if (character == L'\\') {
            ++backslashes;
            continue;
        }
        if (character == L'"') {
            result.append(backslashes * 2 + 1, L'\\');
            result.push_back(L'"');
            backslashes = 0;
            continue;
        }
        result.append(backslashes, L'\\');
        backslashes = 0;
        result.push_back(character);
    }
    result.append(backslashes * 2, L'\\');
    result.push_back(L'"');
    return result;
}

std::wstring BuildActivationArguments(const std::vector<std::wstring>& paths) {
    // GPUI 的正式 CLI 只接收文件/目录；不能发送旧 Tauri 专用标记。
    std::wstring arguments;
    for (const std::wstring& path : paths) {
        if (!arguments.empty()) { arguments.push_back(L' '); }
        arguments.append(QuoteCommandLineArgument(path));
    }
    return arguments;
}

NomoLocale ResolveNomoLocale(std::wstring_view languageTag) noexcept {
    const auto normalized = languageTag;
    if (StartsWith(normalized, L"zh-hant") || StartsWith(normalized, L"zh-tw") ||
        StartsWith(normalized, L"zh-hk") || StartsWith(normalized, L"zh-mo")) {
        return NomoLocale::TraditionalChinese;
    }
    if (StartsWith(normalized, L"zh")) {
        return NomoLocale::SimplifiedChinese;
    }
    if (StartsWith(normalized, L"ja")) {
        return NomoLocale::Japanese;
    }
    return NomoLocale::English;
}

const wchar_t* GetNomoCommandTitle(NomoCommandKind kind, NomoLocale locale) noexcept {
    if (kind == NomoCommandKind::Folder) {
        switch (locale) {
            case NomoLocale::SimplifiedChinese:
                return L"在 Nomo 中打开文件夹";
            case NomoLocale::TraditionalChinese:
                return L"在 Nomo 中開啟資料夾";
            case NomoLocale::Japanese:
                return L"Nomo でフォルダーを開く";
            case NomoLocale::English:
            default:
                return L"Open folder in Nomo";
        }
    }

    switch (locale) {
        case NomoLocale::SimplifiedChinese:
            return L"使用 Nomo 打开";
        case NomoLocale::TraditionalChinese:
            return L"使用 Nomo 開啟";
        case NomoLocale::Japanese:
            return L"Nomo で開く";
        case NomoLocale::English:
        default:
            return L"Open with Nomo";
    }
}

