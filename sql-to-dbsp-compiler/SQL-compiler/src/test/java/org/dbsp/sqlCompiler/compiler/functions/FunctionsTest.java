package org.dbsp.sqlCompiler.compiler.functions;

import org.dbsp.sqlCompiler.compiler.postgres.PostgresBaseTest;
import org.junit.Ignore;
import org.junit.Test;

public class FunctionsTest extends PostgresBaseTest {
    @Test
    public void testLeft() {
        this.q("SELECT LEFT('string', 1);\n" +
                "result\n" +
                "---------\n" +
                " s");
        this.q("SELECT LEFT('string', 0);\n" +
                "result\n" +
                "---------\n" +
                " ");
        this.q("SELECT LEFT('string', 100);\n" +
                "result\n" +
                "---------\n" +
                " string");
        this.q("SELECT LEFT('string', -2);\n" +
                "result\n" +
                "---------\n" +
                " ");
    }

    @Test @Ignore("Bug in Calcite https://issues.apache.org/jira/browse/CALCITE-5859")
    public void testLeftNull() {
        this.q("SELECT LEFT(NULL, 100);\n" +
                "result\n" +
                "---------\n" +
                "NULL");
    }

    @Test
    public void testConcat() {
        this.q("SELECT CONCAT('string', 1);\n" +
                "result\n" +
                "---------\n" +
                " string1");
        this.q("SELECT CONCAT('string', 1, true);\n" +
                "result\n" +
                "---------\n" +
                " string1TRUE");
    }

    @Test
    public void testCoalesce() {
        this.q("SELECT COALESCE(NULL, 5);\n" +
                "result\n" +
                "------\n" +
                "5");
    }
}
