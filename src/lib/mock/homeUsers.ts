import type { AppError, HomeUser, HomeUsers } from "../types";

/** The PIN of the mock Home user who has one. */
export const MOCK_HOME_PIN = "1234";

const avatar = (n: number) => `/ext/https://plex.tv/users/mock${n}/avatar?c=1`;

/**
 * A Plex Home of three: the user who signed in and two others, one with a
 * PIN. Switching swaps who is current (the real app restarts as them).
 */
export class MockHomeUsers {
  private users: HomeUser[] = [
    { uuid: "home-owner", name: "listener", avatar: avatar(0), admin: true, restricted: false, protected: false, current: true },
    { uuid: "home-sam", name: "Sam", avatar: avatar(1), admin: false, restricted: false, protected: true, current: false },
    { uuid: "home-kids", name: "Kids", avatar: null, admin: false, restricted: true, protected: false, current: false },
  ];
  remember = true;
  /** Test hook: the account isn't a Plex Home. */
  notAHome = false;
  /** Every switch asked for, PIN included only as "had a PIN". */
  readonly switches: { uuid: string; withPin: boolean }[] = [];

  list(): HomeUsers {
    return { users: this.notAHome ? [] : this.users.map((u) => ({ ...u })), remember: this.remember };
  }

  current(): HomeUser {
    return this.users.find((u) => u.current) ?? this.users[0];
  }

  /** Throws an AppError like the core for a wrong or missing PIN. */
  switchTo(uuid: string, pin?: string): HomeUser {
    const to = this.users.find((u) => u.uuid === uuid);
    if (!to) throw { kind: "notFound", message: "That Home user isn't in your Plex Home anymore." } satisfies AppError;
    if (pin != null && !/^\d{4}$/.test(pin)) throw { kind: "invalid", message: "A PIN is 4 digits." } satisfies AppError;
    this.switches.push({ uuid, withPin: pin != null });
    if (to.protected && pin !== MOCK_HOME_PIN) {
      throw { kind: "invalid", message: pin == null ? "Enter this user's PIN." : "That PIN isn't right." } satisfies AppError;
    }
    for (const u of this.users) u.current = u.uuid === uuid;
    return to;
  }
}
