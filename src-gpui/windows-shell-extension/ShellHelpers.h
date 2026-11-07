#pragma once

#include <guiddef.h>
#include <string>
#include <string_view>
#include <vector>

inline constexpr std::size_t NomoMaximumSelection = 64;
inline constexpr std::size_t NomoMaximumArguments = 30 * 1024;

inline constexpr GUID CLSID_NomoMarkdownCommand{
    0xbc62b998, 0x15b3, 0x4a1a, {0xa4, 0xec, 0x35, 0xbf, 0xcf, 0x65, 0x2d, 0x70}};
inline constexpr GUID CLSID_NomoFolderCommand{
    0x345a9e7e, 0x91cd, 0x4c79, {0xb6, 0x9c, 0xef, 0xc7, 0xcf, 0x8e, 0x84, 0x08}};

enum class NomoCommandKind {
    Markdown,
    Folder,
};

enum class NomoLocale {
    SimplifiedChinese,
    TraditionalChinese,
    English,
    Japanese,
};

struct NomoSelectionItem {
    std::wstring path;
    bool isFolder;
};

bool IsMarkdownPath(std::wstring_view path) noexcept;
bool IsNomoSelectionSupported(
    NomoCommandKind kind,
    const std::vector<NomoSelectionItem>& items) noexcept;
std::wstring QuoteCommandLineArgument(std::wstring_view argument);
std::wstring BuildActivationArguments(const std::vector<std::wstring>& paths);
NomoLocale ResolveNomoLocale(std::wstring_view languageTag) noexcept;
const wchar_t* GetNomoCommandTitle(NomoCommandKind kind, NomoLocale locale) noexcept;
