import {
  Ban,
  Bookmark,
  CircleCheck,
  Eye,
  EyeOff,
  ListFilter,
  SlidersHorizontal,
  Sparkles,
  ArrowDown,
  ArrowUp,
  CircleMinus,
  Copy,
  CopyPlus,
  Disc3,
  Heart,
  HeartOff,
  Link,
  ListEnd,
  ListMinus,
  ListPlus,
  MessageSquareText,
  MicVocal,
  Pause,
  Pencil,
  Pin,
  PinOff,
  Play,
  Plus,
  Share2,
  Captions,
  Trash2,
  UserRound,
  UserCheck,
  UserPlus,
  UsersRound,
  type LucideIcon,
} from "lucide-react";

/**
 * The leading icon for a menu action, picked from its label so the same
 * action shows the same icon in every menu. First match wins.
 */
const RULES: [RegExp, LucideIcon][] = [
  [/^Smart playlist$/, ListFilter],
  [/^Prompted playlist$/, Sparkles],
  [/^Describe a playlist$/, MessageSquareText],
  [/^Hide song$/, EyeOff],
  [/^Unhide song$/, Eye],
  [/^Don't play this artist$/, Ban],
  [/^Allow this artist$/, CircleCheck],
  [/^Bookmark/, Bookmark],
  [/^Edit rules$/, SlidersHorizontal],
  [/^Play$/, Play],
  [/^Pause$/, Pause],
  [/^Add (popular songs )?to queue$/, ListEnd],
  [/^Remove from queue$/, ListMinus],
  [/^Add to (other )?playlist$/, ListPlus],
  [/^Add to the session's queue$/, ListEnd],
  [/^Start a listening session$/, UsersRound],
  [/^Just show new albums$/, Eye],
  [/^Follow$/, UserPlus],
  [/^Unfollow$/, UserCheck],
  [/^Remove from this playlist$/, CircleMinus],
  [/^Add to Liked Songs$/, Heart],
  [/^Remove from Liked Songs$/, HeartOff],
  [/^Go to album$/, Disc3],
  [/^Shuffle albums$/, Disc3],
  [/^Go to /, UserRound],
  [/^(Add|Edit) lyrics$/, MicVocal],
  [/^Transcribe/, Captions],
  [/^Share$/, Share2],
  [/^Copy .*link/, Link],
  [/^Copy /, Copy],
  [/^Edit details$/, Pencil],
  [/^Rename$/, Pencil],
  [/^Delete$/, Trash2],
  [/^Duplicate$/, CopyPlus],
  [/^Create playlist$/, Plus],
  [/^Unpin/, PinOff],
  [/^Pin/, Pin],
  [/^Move up$/, ArrowUp],
  [/^Move down$/, ArrowDown],
];

export function menuIconFor(label: string): LucideIcon | null {
  for (const [re, icon] of RULES) if (re.test(label)) return icon;
  return null;
}

/** Size and stroke shared by every menu icon. */
export const MENU_ICON_SIZE = 16;
export const MENU_ICON_STROKE = 2;
