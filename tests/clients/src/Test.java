import java.sql.*;

public class Test {
    public static void main(String[] args) {
        String pgPort = System.getenv("PG_PORT");
        String myPort = System.getenv("MY_PORT");

        // PG test
        try {
            Connection conn = DriverManager.getConnection(
                "jdbc:postgresql://127.0.0.1:" + pgPort + "/app?sslmode=disable",
                "user", "pass");
            Statement stmt = conn.createStatement();
            ResultSet rs = stmt.executeQuery("SELECT 1");
            rs.next();
            if (rs.getInt(1) != 1) throw new RuntimeException("unexpected");
            conn.close();
            System.out.println("PG:PASS");
        } catch (Exception e) {
            System.out.println("PG:FAIL:" + e.getMessage());
        }

        // MySQL test
        try {
            Connection conn = DriverManager.getConnection(
                "jdbc:mysql://127.0.0.1:" + myPort + "/myapp?sslMode=DISABLED&allowPublicKeyRetrieval=true",
                "root", "");
            Statement stmt = conn.createStatement();
            ResultSet rs = stmt.executeQuery("SELECT 1");
            rs.next();
            if (rs.getInt(1) != 1) throw new RuntimeException("unexpected");
            conn.close();
            System.out.println("MySQL:PASS");
        } catch (Exception e) {
            System.out.println("MySQL:FAIL:" + e.getMessage());
        }
    }
}
