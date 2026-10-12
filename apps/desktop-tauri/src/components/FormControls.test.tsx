import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Field, NumberInput, Select, Toggle } from "./FormControls";

const noop = () => {};

describe("Field", () => {
  it("names its control after the field label and description", () => {
    render(
      <>
        <Field label="Start at Login" description="Launch after sign-in">
          <Toggle checked={false} onChange={noop} />
        </Field>
        <Field label="Theme">
          <Select value="auto" options={[{ value: "auto", label: "Auto" }]} onChange={noop} />
        </Field>
        <Field label="Refresh interval">
          <NumberInput value={5} onChange={noop} />
        </Field>
      </>,
    );

    const toggle = screen.getByRole("checkbox", { name: "Start at Login" });
    expect(toggle).toHaveAccessibleDescription("Launch after sign-in");
    expect(screen.getByRole("combobox", { name: "Theme" })).toBeInTheDocument();
    expect(screen.getByRole("spinbutton", { name: "Refresh interval" })).toBeInTheDocument();
  });

  it("keeps an explicit aria-label over the field label", () => {
    render(
      <Field label="Thresholds">
        <NumberInput value={80} onChange={noop} ariaLabel="High threshold" />
      </Field>,
    );

    expect(screen.getByRole("spinbutton", { name: "High threshold" })).toBeInTheDocument();
  });
});
