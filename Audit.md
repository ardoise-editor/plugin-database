# Audit du plugin Database

Chaque point est retiré de ce fichier quand il est traité.

13. Autres moteurs — il reste SQL Server (DuckDB est fait).
    - Aucun code SQL Server n'est écrit.
    - À prévoir : un driver `drivers/mssql` (bibliothèque `tiberius`), le type de source,
      et la pagination propre à T-SQL (`OFFSET … FETCH`, noms entre crochets) : la partie
      commune des drivers écrit aujourd'hui `LIMIT`.
    - Pour le tester : un serveur de test Docker, dont la licence Microsoft (`ACCEPT_EULA=Y`)
      doit être acceptée par l'utilisateur.
