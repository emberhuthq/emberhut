import { describe, expect, it } from "vitest";
import { emberhutLink } from "./emberhutLink";

describe("emberhutLink", () => {
  it("reads the app's own links", () => {
    expect(emberhutLink("emberhut://task/3744")).toEqual({ screen: "Task", id: 3744 });
    expect(emberhutLink("emberhut://project/4")).toEqual({ screen: "Project", id: 4 });
    expect(emberhutLink("emberhut://session/r-ab%3A1")).toEqual({ screen: "Session", id: "r-ab:1" });
    expect(emberhutLink("emberhut://browser")).toEqual({ screen: "Browser" });
    expect(emberhutLink("emberhut://party")).toEqual({ screen: "Parties" });
    expect(emberhutLink("emberhut://party/7")).toEqual({ screen: "Party", id: 7 });
    expect(emberhutLink("emberhut://approvals")).toEqual({ screen: "Approvals" });
    expect(emberhutLink("emberhut://approval/12")).toEqual({ screen: "Approval", id: 12 });
  });

  it("leaves other links to the system", () => {
    expect(emberhutLink("https://github.com/x")).toBeNull();
    expect(emberhutLink("other://task/1")).toBeNull();
    expect(emberhutLink("emberhut://task/")).toBeNull();
    expect(emberhutLink("emberhut://task/abc")).toBeNull();
    expect(emberhutLink("emberhut://party/x")).toBeNull();
    expect(emberhutLink("emberhut://approval")).toBeNull();
    expect(emberhutLink("emberhut://approvalsfoo")).toBeNull();
    expect(emberhutLink("emberhut://approvals/7")).toBeNull();
    expect(emberhutLink("emberhut://taskx/7")).toBeNull();
    expect(emberhutLink("emberhut://approvals?from=slack")).toEqual({ screen: "Approvals" });
  });
});
