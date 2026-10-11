import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProvidersSidebar, type ProviderSidebarRow } from "./ProvidersSidebar";

vi.mock("../../../hooks/useLocale", () => ({
  useLocale: () => ({ t: (key: string) => key }),
}));

function rows(): ProviderSidebarRow[] {
  return [
    { id: "codex", displayName: "Codex", enabled: true, status: "ok" },
    { id: "claude", displayName: "Claude", enabled: true, status: "error" },
    { id: "cursor", displayName: "Cursor", enabled: false, status: "disabled" },
  ];
}

function renderSidebar(overrides: Partial<Parameters<typeof ProvidersSidebar>[0]> = {}) {
  const props = {
    providers: rows(),
    selectedId: "codex",
    canReorder: true,
    onSelect: vi.fn(),
    onReorder: vi.fn(),
    onToggleEnabled: vi.fn(),
    ...overrides,
  };
  const view = render(<ProvidersSidebar {...props} />);
  return { ...props, ...view };
}

const option = (name: string) => screen.getByRole("option", { name: new RegExp(`^${name} `) });

describe("ProvidersSidebar", () => {
  it("shows a status dot on enabled rows only and dims disabled rows", () => {
    renderSidebar();
    expect(screen.getAllByRole("option")).toHaveLength(3);
    expect(option("Codex").querySelector(".providers-sidebar__status--ok")).not.toBeNull();
    expect(option("Claude").querySelector(".providers-sidebar__status--error")).not.toBeNull();
    const cursor = option("Cursor");
    expect(cursor.querySelector(".providers-sidebar__status")).toBeNull();
    expect(cursor).toHaveClass("providers-sidebar__row--disabled");
    expect(option("Codex")).toHaveAttribute("aria-selected", "true");
  });

  it("says when no provider matches", () => {
    renderSidebar({ providers: [], selectedId: null });
    expect(screen.getByText("ProviderSidebarNoMatches")).toBeInTheDocument();
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });

  it("selects with the arrow keys and moves with Alt+Arrow", () => {
    const { onSelect, onReorder } = renderSidebar();
    fireEvent.keyDown(option("Codex"), { key: "ArrowDown" });
    expect(onSelect).toHaveBeenCalledWith("claude");
    expect(onReorder).not.toHaveBeenCalled();

    fireEvent.keyDown(option("Codex"), { key: "ArrowDown", altKey: true });
    expect(onReorder).toHaveBeenCalledWith(["claude", "codex", "cursor"]);
  });

  it("keeps Alt+Arrow and drag inert while the list cannot be reordered", () => {
    const { onReorder } = renderSidebar({ canReorder: false });
    fireEvent.keyDown(option("Codex"), { key: "ArrowDown", altKey: true });
    expect(onReorder).not.toHaveBeenCalled();
    expect(option("Codex")).toHaveAttribute("draggable", "false");
  });

  it("toggles a provider from the row context menu", () => {
    const { onToggleEnabled } = renderSidebar();
    fireEvent.contextMenu(option("Claude"), { clientX: 40, clientY: 60 });
    const menu = screen.getByRole("menu");
    fireEvent.click(within(menu).getByRole("menuitem", { name: "ProviderContextDisable" }));
    expect(onToggleEnabled).toHaveBeenCalledWith("claude", false);
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.contextMenu(option("Cursor"));
    fireEvent.click(screen.getByRole("menuitem", { name: "ProviderContextEnable" }));
    expect(onToggleEnabled).toHaveBeenCalledWith("cursor", true);
  });

  it("moves a provider from the context menu and disables impossible moves", () => {
    const { onReorder } = renderSidebar();
    fireEvent.contextMenu(option("Codex"));
    expect(screen.getByRole("menuitem", { name: "ProviderSidebarMoveUp" })).toBeDisabled();
    fireEvent.click(screen.getByRole("menuitem", { name: "ProviderSidebarMoveDown" }));
    expect(onReorder).toHaveBeenCalledWith(["claude", "codex", "cursor"]);
  });

  it("disables Move Up and Move Down while sorted or searching", () => {
    renderSidebar({ canReorder: false });
    fireEvent.contextMenu(option("Claude"));
    expect(screen.getByRole("menuitem", { name: "ProviderSidebarMoveUp" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "ProviderSidebarMoveDown" })).toBeDisabled();
    expect(screen.getByRole("menuitem", { name: "ProviderContextDisable" })).toBeEnabled();
  });

  it("opens the menu from the keyboard and closes it with Escape", () => {
    renderSidebar();
    const claude = option("Claude");
    fireEvent.keyDown(claude, { key: "F10", shiftKey: true });
    const menu = screen.getByRole("menu");
    expect(menu).toBeInTheDocument();
    fireEvent.keyDown(menu, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(claude).toHaveFocus();

    fireEvent.keyDown(claude, { key: "ContextMenu" });
    expect(screen.getByRole("menu")).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("reorders by drag and drop", () => {
    const { onReorder } = renderSidebar();
    const store = new Map<string, string>();
    const dataTransfer = {
      setData: (k: string, v: string) => store.set(k, v),
      getData: (k: string) => store.get(k) ?? "",
      effectAllowed: "",
      dropEffect: "",
    };
    fireEvent.dragStart(option("Cursor"), { dataTransfer });
    fireEvent.dragOver(option("Codex"), { dataTransfer });
    fireEvent.drop(option("Codex"), { dataTransfer });
    expect(onReorder).toHaveBeenCalledWith(["cursor", "codex", "claude"]);
  });
});
