export type TurnEndResult = {
  reason?: string;
  turn?: number;
};

export interface GetTurnEndQuery {
  sessionId?: string;
}
