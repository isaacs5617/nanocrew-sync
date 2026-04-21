import React from 'react';

interface AuthCtx {
  token: string;
  signOut: () => void;
}

export const AuthContext = React.createContext<AuthCtx>({ token: '', signOut: () => {} });

export function useAuth(): AuthCtx {
  return React.useContext(AuthContext);
}
