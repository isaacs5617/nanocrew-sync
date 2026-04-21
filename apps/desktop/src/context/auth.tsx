import React from 'react';

interface AuthCtx {
  token: string;
  signOut: () => void;
  lock: () => void;
}

export const AuthContext = React.createContext<AuthCtx>({
  token: '',
  signOut: () => {},
  lock: () => {},
});

export function useAuth(): AuthCtx {
  return React.useContext(AuthContext);
}
